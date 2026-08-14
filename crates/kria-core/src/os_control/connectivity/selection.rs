//! Connectivity backend selection and captured-argv construction.
//!
//! linux-os-control-production **Task 2.3** (OSC-015, OSC-031), design §9.4.
//!
//! NetworkManager's D-Bus API (`org.freedesktop.NetworkManager`) is the
//! preferred, authoritative provider. Its `nmcli` CLI front-end is retained as
//! a **declared degraded** structured-command fallback — the same relationship
//! `os_control::audio`/`os_control::display` have with their CLI backends —
//! until the live D-Bus transport is wired by a desktop composition root.

use crate::os_control::contract::Digest;
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::OsControlError;

/// The concrete host connectivity backend a provider selected. The string form
/// is kept **compatible with the pre-migration handlers**, which reported no
/// explicit backend field; `"nmcli"` is the fallback CLI, `"network_manager"`
/// is the (not-yet-wired) native D-Bus path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectivityBackend {
    /// NetworkManager D-Bus (`org.freedesktop.NetworkManager`). Preferred.
    NetworkManager,
    /// `nmcli` structured-command fallback. Degraded.
    Nmcli,
}

impl ConnectivityBackend {
    /// The full, ordered preference list (most preferred first).
    pub const PREFERENCE: [ConnectivityBackend; 2] = [
        ConnectivityBackend::NetworkManager,
        ConnectivityBackend::Nmcli,
    ];

    /// The stable label used in the `backend` result field and traces.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ConnectivityBackend::NetworkManager => "network_manager",
            ConnectivityBackend::Nmcli => "nmcli",
        }
    }

    /// Whether this backend is a declared **degraded** provider (not the
    /// preferred authoritative D-Bus path).
    #[must_use]
    pub fn is_degraded(self) -> bool {
        !matches!(self, ConnectivityBackend::NetworkManager)
    }

    /// The trusted absolute executable path for this backend's structured
    /// command (only the `nmcli` fallback dispatches through a process).
    #[must_use]
    fn executable_path(self) -> &'static str {
        match self {
            ConnectivityBackend::NetworkManager => "/usr/bin/nmcli",
            ConnectivityBackend::Nmcli => "/usr/bin/nmcli",
        }
    }

    /// A stable trusted-executable identity used by the fallback adapter. Live
    /// transports compare the on-disk identity against this to detect drift; the
    /// deny-live provider tests use it directly.
    #[must_use]
    pub fn trusted_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            self.executable_path(),
            Digest::of_str(&format!("{}-fallback-v1", self.as_str())),
        )
    }
}

/// Select the most-preferred available backend, or `None` when no session
/// connectivity backend is present (→ the provider reports `Unavailable`).
#[must_use]
pub fn select_backend(available: &[ConnectivityBackend]) -> Option<ConnectivityBackend> {
    ConnectivityBackend::PREFERENCE
        .into_iter()
        .find(|candidate| available.contains(candidate))
}

/// The argv for reading the Wi-Fi radio enabled/disabled state.
#[must_use]
pub fn query_radio_argv() -> Vec<String> {
    vec!["radio".into(), "wifi".into()]
}

/// The argv for setting the Wi-Fi radio enabled/disabled state.
#[must_use]
pub fn set_radio_argv(enabled: bool) -> Vec<String> {
    vec![
        "radio".into(),
        "wifi".into(),
        if enabled { "on".into() } else { "off".into() },
    ]
}

/// The argv for reading the currently active SSID.
#[must_use]
pub fn query_active_ssid_argv() -> Vec<String> {
    vec![
        "-t".into(),
        "-f".into(),
        "ACTIVE,SSID".into(),
        "device".into(),
        "wifi".into(),
        "list".into(),
    ]
}

/// The argv for scanning available Wi-Fi networks.
#[must_use]
pub fn scan_wifi_argv() -> Vec<String> {
    vec![
        "-t".into(),
        "-f".into(),
        "SSID,BSSID,SIGNAL,SECURITY".into(),
        "device".into(),
        "wifi".into(),
        "list".into(),
    ]
}

/// The argv for connecting to a Wi-Fi network by SSID. The password (when
/// present) is appended as a literal, non-shell-interpreted argv element and is
/// marked secret in the request's [`crate::os_control::linux::structured_command::RedactionMap`]
/// so it never appears in a redacted summary/trace/audit projection.
#[must_use]
pub fn connect_wifi_argv(ssid: &str, has_password: bool) -> Vec<String> {
    let mut args = vec![
        "device".into(),
        "wifi".into(),
        "connect".into(),
        ssid.to_string(),
    ];
    if has_password {
        // The caller substitutes the real password value at the same index
        // before dispatch; this shape only documents argv layout.
        args.push("password".into());
    }
    args
}

/// The argv for listing known network devices (Task 3.5).
#[must_use]
pub fn list_devices_argv() -> Vec<String> {
    vec![
        "-t".into(),
        "-f".into(),
        "DEVICE,TYPE,STATE".into(),
        "device".into(),
        "status".into(),
    ]
}

/// The argv for listing saved network profiles (Task 3.5).
#[must_use]
pub fn list_profiles_argv() -> Vec<String> {
    vec![
        "-t".into(),
        "-f".into(),
        "NAME,UUID,TYPE,DEVICE".into(),
        "connection".into(),
        "show".into(),
    ]
}

/// The argv for disconnecting a device from its current Wi-Fi connection
/// (Task 3.5, `disconnect_wifi`). `device` is the trusted stable device
/// identity string (never taken verbatim from unvalidated model input).
#[must_use]
pub fn disconnect_wifi_argv(device: &str) -> Vec<String> {
    vec!["device".into(), "disconnect".into(), device.to_string()]
}

/// The argv for forgetting (deleting) a saved profile by UUID (Task 3.5,
/// `forget_wifi`).
#[must_use]
pub fn forget_profile_argv(profile: &str) -> Vec<String> {
    vec!["connection".into(), "delete".into(), profile.to_string()]
}

/// The argv for activating an existing saved profile — Wi-Fi or Ethernet —
/// optionally bound to a specific device (Task 3.5,
/// `activate_network_profile`).
#[must_use]
pub fn activate_profile_argv(profile: &str, device: Option<&str>) -> Vec<String> {
    let mut args = vec!["connection".into(), "up".into(), profile.to_string()];
    if let Some(device) = device {
        args.push("ifname".into());
        args.push(device.to_string());
    }
    args
}

// ─────────────────────────────────────────────────────────────────────────────
// Tasks 4.2 / 5.3 / 5.6 — VPN, hotspot, proxy and saved-credential argv
// ─────────────────────────────────────────────────────────────────────────────

/// The NetworkManager property holding a Wi-Fi profile's pre-shared key.
pub const WIFI_PSK_PROPERTY: &str = "802-11-wireless-security.psk";
/// The NetworkManager property holding a Wi-Fi profile's key-management mode.
/// An empty value means the profile is **open** (no encryption).
pub const WIFI_KEY_MGMT_PROPERTY: &str = "802-11-wireless-security.key-mgmt";
/// The NetworkManager property holding a Wi-Fi profile's radio mode. `ap` is a
/// hotspot (access point) profile.
pub const WIFI_MODE_PROPERTY: &str = "802-11-wireless.mode";
/// The NetworkManager property holding a VPN profile's secrets.
pub const VPN_SECRETS_PROPERTY: &str = "vpn.secrets";

/// The `802-11-wireless.mode` value that denotes an access-point (hotspot)
/// profile.
pub const WIFI_MODE_ACCESS_POINT: &str = "ap";

/// The inclusive byte bounds of a WPA/WPA2 pre-shared key passphrase. A shorter
/// passphrase is not accepted by the standard and must never be silently
/// widened into an open or trivially crackable access point.
pub const WPA_PASSPHRASE_MIN_BYTES: usize = 8;
/// The upper inclusive bound of a WPA/WPA2 passphrase.
pub const WPA_PASSPHRASE_MAX_BYTES: usize = 63;

/// The literal path `nmcli`'s `passwd-file` option is pointed at so the secret
/// arrives on the child's **stdin** rather than in argv or a temporary file.
/// Only this fixed, non-secret token ever enters the argv digest.
pub const SECRET_STDIN_PATH: &str = "/dev/stdin";

/// Reject a caller-supplied value before it becomes an argv element.
///
/// Rejects rather than escapes (there is no shell to escape for): an empty
/// value, a control character, and a leading `-` that a tool would read as an
/// option.
pub fn validate_argv_token(field: &str, value: &str) -> Result<(), OsControlError> {
    use crate::os_control::contract::{SafeField, SafeText};
    let reject = |reason: &str| OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    };
    if value.is_empty() {
        return Err(reject("value must not be empty"));
    }
    if value.starts_with('-') {
        return Err(reject(
            "value must not start with '-': it would be read as a command-line option",
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(reject("value must not contain control characters"));
    }
    Ok(())
}

/// The argv for listing currently active connections with their profile UUID,
/// type, activation state and bound device (Tasks 4.2 / 5.3).
#[must_use]
pub fn list_active_argv() -> Vec<String> {
    vec![
        "-t".into(),
        "-f".into(),
        "UUID,TYPE,STATE,DEVICE".into(),
        "connection".into(),
        "show".into(),
        "--active".into(),
    ]
}

/// The argv for reading one property of a saved profile, addressed by **UUID**.
///
/// `--show-secrets` is deliberately never passed, so a secret-valued property
/// reports `<hidden>` and the real credential never reaches this process.
#[must_use]
pub fn query_profile_property_argv(property: &str, uuid: &str) -> Vec<String> {
    vec![
        "-t".into(),
        "-f".into(),
        property.to_string(),
        "connection".into(),
        "show".into(),
        "uuid".into(),
        uuid.to_string(),
    ]
}

/// The argv for activating a saved profile by **UUID** (VPN or hotspot).
///
/// When `secret_on_stdin` is set, `passwd-file /dev/stdin` is appended so the
/// activation credential travels on the child's stdin: argv carries only the
/// fixed [`SECRET_STDIN_PATH`] token, never the credential.
#[must_use]
pub fn profile_up_argv(uuid: &str, ifname: Option<&str>, secret_on_stdin: bool) -> Vec<String> {
    let mut args = vec![
        "connection".into(),
        "up".into(),
        "uuid".into(),
        uuid.to_string(),
    ];
    if let Some(ifname) = ifname {
        args.push("ifname".into());
        args.push(ifname.to_string());
    }
    if secret_on_stdin {
        args.push("passwd-file".into());
        args.push(SECRET_STDIN_PATH.into());
    }
    args
}

/// The argv for deactivating a saved profile by **UUID** (VPN or hotspot).
#[must_use]
pub fn profile_down_argv(uuid: &str) -> Vec<String> {
    vec![
        "connection".into(),
        "down".into(),
        "uuid".into(),
        uuid.to_string(),
    ]
}

/// The argv for `nmcli`'s connection editor, addressed by **UUID**.
///
/// The editor takes its commands on stdin, which is the only `nmcli` path that
/// writes a stored credential without placing it in argv. The secret therefore
/// never enters the argv digest, the audit record, or `/proc/<pid>/cmdline`.
#[must_use]
pub fn profile_edit_argv(uuid: &str) -> Vec<String> {
    vec![
        "connection".into(),
        "edit".into(),
        "uuid".into(),
        uuid.to_string(),
    ]
}

/// The argv for clearing a stored credential property on a profile addressed by
/// **UUID**. No secret is involved: the value written is the empty string.
#[must_use]
pub fn clear_profile_secret_argv(uuid: &str, property: &str) -> Vec<String> {
    vec![
        "connection".into(),
        "modify".into(),
        "uuid".into(),
        uuid.to_string(),
        property.to_string(),
        String::new(),
    ]
}

/// The `passwd-file` body `nmcli` expects, delivered on stdin: one
/// `setting.property:value` line. Returned as bytes so the credential is never
/// held in a `String` that could be formatted into a log.
#[must_use]
pub fn passwd_file_body(property: &str, secret: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(property.len() + secret.len() + 2);
    body.extend_from_slice(property.as_bytes());
    body.push(b':');
    body.extend_from_slice(secret);
    body.push(b'\n');
    body
}

/// The `nmcli connection edit` script that sets one property to `secret` and
/// saves, delivered on stdin.
#[must_use]
pub fn editor_set_secret_script(property: &str, secret: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(property.len() + secret.len() + 24);
    body.extend_from_slice(b"set ");
    body.extend_from_slice(property.as_bytes());
    body.push(b' ');
    body.extend_from_slice(secret);
    body.extend_from_slice(b"\nsave\nquit\n");
    body
}

/// The argv for reading NetworkManager's own connectivity verdict (Task 4.2).
/// `STATE` is requested alongside `CONNECTIVITY` so an `unknown` verdict is
/// distinguishable from a manager that is not running at all.
#[must_use]
pub fn query_connectivity_argv() -> Vec<String> {
    vec![
        "-t".into(),
        "-f".into(),
        "STATE,CONNECTIVITY".into(),
        "general".into(),
    ]
}

/// The argv for reading one device's IP configuration (Task 4.2). Only presence
/// is retained by the parser — no address, gateway or resolver value is kept.
#[must_use]
pub fn query_device_ip_argv(device: &str) -> Vec<String> {
    vec![
        "-t".into(),
        "-f".into(),
        "GENERAL.STATE,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,IP4.ROUTE".into(),
        "device".into(),
        "show".into(),
        device.to_string(),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Desktop proxy backend (Task 5.3)
// ─────────────────────────────────────────────────────────────────────────────

/// The GSettings schema holding the desktop-wide proxy mode, PAC URI and
/// exclusion list.
pub const PROXY_SCHEMA: &str = "org.gnome.system.proxy";

/// The recognised desktop proxy backend. Separate from
/// [`ConnectivityBackend`] because the system-wide proxy is a desktop setting,
/// not a NetworkManager connection property: NetworkManager's own `proxy.method`
/// cannot express per-protocol manual endpoints at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyBackend {
    /// The GSettings (`org.gnome.system.proxy`) desktop proxy store.
    GSettings,
}

impl ProxyBackend {
    /// The stable label used in a result field.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GSettings => "gsettings",
        }
    }

    /// The trusted absolute executable for this backend.
    #[must_use]
    pub fn trusted_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            match self {
                Self::GSettings => "/usr/bin/gsettings",
            },
            Digest::of_str(&format!("proxy-{}-v1", self.as_str())),
        )
    }
}

/// The argv for reading one desktop proxy key.
#[must_use]
pub fn proxy_get_argv(schema: &str, key: &str) -> Vec<String> {
    vec!["get".into(), schema.to_string(), key.to_string()]
}

/// The argv for writing one desktop proxy key. `value` is a GVariant literal
/// (already quoted for a string, bracketed for an array).
#[must_use]
pub fn proxy_set_argv(schema: &str, key: &str, value: &str) -> Vec<String> {
    vec![
        "set".into(),
        schema.to_string(),
        key.to_string(),
        value.to_string(),
    ]
}

/// Render a GVariant string literal for a `gsettings set`. Rejects a value
/// carrying a quote or a control character rather than escaping it.
pub fn gvariant_string(field: &str, value: &str) -> Result<String, OsControlError> {
    use crate::os_control::contract::{SafeField, SafeText};
    if value.contains('\'') || value.contains('\\') || value.chars().any(char::is_control) {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new(field),
            reason: SafeText::new("value must not contain a quote, backslash or control character"),
        });
    }
    Ok(format!("'{value}'"))
}

/// Render a GVariant string-array literal for a `gsettings set`.
pub fn gvariant_string_list(field: &str, values: &[String]) -> Result<String, OsControlError> {
    let rendered = values
        .iter()
        .map(|value| gvariant_string(field, value))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!("[{}]", rendered.join(", ")))
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn selection_matrix_prefers_network_manager_then_nmcli() {
        use ConnectivityBackend::*;
        let cases: &[(&[ConnectivityBackend], Option<ConnectivityBackend>)] = &[
            (&[NetworkManager, Nmcli], Some(NetworkManager)),
            (&[Nmcli], Some(Nmcli)),
            (&[], None),
        ];
        for (available, expected) in cases {
            assert_eq!(
                select_backend(available),
                *expected,
                "available {available:?}"
            );
        }
    }

    #[test]
    fn degraded_classification() {
        assert!(!ConnectivityBackend::NetworkManager.is_degraded());
        assert!(ConnectivityBackend::Nmcli.is_degraded());
    }

    #[test]
    fn captured_argv_golden() {
        assert_eq!(query_radio_argv(), vec!["radio", "wifi"]);
        assert_eq!(set_radio_argv(true), vec!["radio", "wifi", "on"]);
        assert_eq!(set_radio_argv(false), vec!["radio", "wifi", "off"]);
        assert_eq!(
            query_active_ssid_argv(),
            vec!["-t", "-f", "ACTIVE,SSID", "device", "wifi", "list"]
        );
        assert_eq!(
            scan_wifi_argv(),
            vec![
                "-t",
                "-f",
                "SSID,BSSID,SIGNAL,SECURITY",
                "device",
                "wifi",
                "list"
            ]
        );
        assert_eq!(
            connect_wifi_argv("MyNet", true),
            vec!["device", "wifi", "connect", "MyNet", "password"]
        );
        assert_eq!(
            connect_wifi_argv("MyNet", false),
            vec!["device", "wifi", "connect", "MyNet"]
        );
        assert_eq!(
            list_devices_argv(),
            vec!["-t", "-f", "DEVICE,TYPE,STATE", "device", "status"]
        );
        assert_eq!(
            list_profiles_argv(),
            vec!["-t", "-f", "NAME,UUID,TYPE,DEVICE", "connection", "show"]
        );
        assert_eq!(
            disconnect_wifi_argv("wlan0"),
            vec!["device", "disconnect", "wlan0"]
        );
        assert_eq!(
            forget_profile_argv("11111111-1111-1111-1111-111111111111"),
            vec![
                "connection",
                "delete",
                "11111111-1111-1111-1111-111111111111"
            ]
        );
        assert_eq!(
            activate_profile_argv("HomeNet", None),
            vec!["connection", "up", "HomeNet"]
        );
        assert_eq!(
            activate_profile_argv("HomeNet", Some("wlan0")),
            vec!["connection", "up", "HomeNet", "ifname", "wlan0"]
        );
    }

    #[test]
    fn trusted_executables_are_absolute_and_valid() {
        for backend in ConnectivityBackend::PREFERENCE {
            let exe = backend
                .trusted_executable()
                .expect("valid trusted executable");
            assert!(exe.path().starts_with('/'));
        }
        let proxy = ProxyBackend::GSettings
            .trusted_executable()
            .expect("valid trusted executable");
        assert!(proxy.path().starts_with('/'));
    }

    // ── Tasks 4.2 / 5.3 / 5.6 ───────────────────────────────────────────────

    const UUID: &str = "11111111-1111-1111-1111-111111111111";

    #[test]
    fn vpn_and_hotspot_argv_golden_addresses_profiles_by_uuid() {
        assert_eq!(
            list_active_argv(),
            vec![
                "-t",
                "-f",
                "UUID,TYPE,STATE,DEVICE",
                "connection",
                "show",
                "--active"
            ]
        );
        // Always `uuid <id>`: a profile NAME is not a unique identity.
        assert_eq!(
            profile_up_argv(UUID, None, false),
            vec!["connection", "up", "uuid", UUID]
        );
        assert_eq!(
            profile_up_argv(UUID, Some("wlan0"), false),
            vec!["connection", "up", "uuid", UUID, "ifname", "wlan0"]
        );
        assert_eq!(
            profile_down_argv(UUID),
            vec!["connection", "down", "uuid", UUID]
        );
        assert_eq!(
            profile_edit_argv(UUID),
            vec!["connection", "edit", "uuid", UUID]
        );
        assert_eq!(
            query_profile_property_argv(WIFI_MODE_PROPERTY, UUID),
            vec![
                "-t",
                "-f",
                "802-11-wireless.mode",
                "connection",
                "show",
                "uuid",
                UUID
            ]
        );
    }

    #[test]
    fn secret_bearing_argv_carries_only_the_fixed_stdin_path() {
        let args = profile_up_argv(UUID, Some("wlan0"), true);
        assert_eq!(
            args,
            vec![
                "connection",
                "up",
                "uuid",
                UUID,
                "ifname",
                "wlan0",
                "passwd-file",
                "/dev/stdin"
            ]
        );
        // The only secret-adjacent argv element is a fixed path, never a value.
        assert!(args.iter().all(|arg| arg != "password"));
    }

    #[test]
    fn profile_query_never_requests_secrets() {
        for property in [WIFI_PSK_PROPERTY, VPN_SECRETS_PROPERTY] {
            let args = query_profile_property_argv(property, UUID);
            assert!(
                !args.iter().any(|a| a == "--show-secrets" || a == "-s"),
                "{property} query must not ask nmcli to reveal secrets"
            );
        }
    }

    #[test]
    fn clear_secret_argv_writes_an_empty_value_and_no_option_like_token() {
        let args = clear_profile_secret_argv(UUID, WIFI_PSK_PROPERTY);
        assert_eq!(
            args,
            vec![
                "connection",
                "modify",
                "uuid",
                UUID,
                "802-11-wireless-security.psk",
                ""
            ]
        );
        // A `-`-prefixed removal token would be read as an option.
        assert!(!args.iter().any(|a| a.starts_with('-')));
    }

    #[test]
    fn stdin_bodies_carry_the_secret_and_the_property_only() {
        let body = passwd_file_body(WIFI_PSK_PROPERTY, b"correct horse");
        assert_eq!(body, b"802-11-wireless-security.psk:correct horse\n".to_vec());
        let script = editor_set_secret_script(WIFI_PSK_PROPERTY, b"correct horse");
        assert_eq!(
            script,
            b"set 802-11-wireless-security.psk correct horse\nsave\nquit\n".to_vec()
        );
    }

    #[test]
    fn argv_token_validation_rejects_rather_than_escapes() {
        assert!(validate_argv_token("device", "wlan0").is_ok());
        assert!(validate_argv_token("device", "").is_err());
        assert!(validate_argv_token("device", "-rf").is_err());
        assert!(validate_argv_token("device", "wlan0\n--help").is_err());
        assert!(validate_argv_token("device", "wlan\t0").is_err());
        // Shell metacharacters are literal (no shell is involved), so they pass.
        assert!(validate_argv_token("device", "wlan0;ls").is_ok());
    }

    #[test]
    fn proxy_argv_golden() {
        assert_eq!(
            proxy_get_argv(PROXY_SCHEMA, "mode"),
            vec!["get", "org.gnome.system.proxy", "mode"]
        );
        assert_eq!(
            proxy_set_argv(PROXY_SCHEMA, "mode", "'manual'"),
            vec!["set", "org.gnome.system.proxy", "mode", "'manual'"]
        );
    }

    #[test]
    fn gvariant_rendering_rejects_quote_injection() {
        assert_eq!(gvariant_string("mode", "manual").unwrap(), "'manual'");
        assert_eq!(
            gvariant_string_list("exclusions", &["localhost".into(), "::1".into()]).unwrap(),
            "['localhost', '::1']"
        );
        assert_eq!(gvariant_string_list("exclusions", &[]).unwrap(), "[]");
        assert!(gvariant_string("mode", "man'ual").is_err());
        assert!(gvariant_string("mode", "man\\ual").is_err());
        assert!(gvariant_string("mode", "man\nual").is_err());
    }

    #[test]
    fn wpa_passphrase_bounds_match_the_standard() {
        assert_eq!(WPA_PASSPHRASE_MIN_BYTES, 8);
        assert_eq!(WPA_PASSPHRASE_MAX_BYTES, 63);
    }
}
