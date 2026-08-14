//! Live NetworkManager D-Bus / `nmcli` connectivity adapter (raw transport
//! seam).
//!
//! linux-os-control-production **Task 2.3** — "Migrate Wi-Fi and power-profile
//! controls" (OSC-015, OSC-031), design §3, §9.4
//! (`linux/providers/network_manager.rs`).
//!
//! # Host safety
//!
//! Driving Wi-Fi (`nmcli`, or a future native NetworkManager D-Bus call) is a
//! **raw live transport**. Like
//! [`crate::os_control::linux::providers::pipewire`] and
//! [`crate::os_control::linux::providers::gnome_display`], this adapter:
//!
//! 1. can be constructed **only** with a
//!    [`crate::os_control::access::LiveHostAccessToken`] (mintable solely in a
//!    live composition root under `os-control-live`), so no completion test can
//!    build it; and
//! 2. calls [`crate::os_control::access::deny_live_transport`] **before** any
//!    read or dispatch, so a deny-live (`os-control-test`) build that reached
//!    here would trip the sentinel and abort rather than run a child process.
//!
//! The live query/launch wiring is composed by the desktop startup root; until
//! then the methods fail closed with [`OsControlError::Unavailable`] and never
//! fall back to an ungoverned subprocess. Deny-live tests inject
//! [`crate::os_control::connectivity::fake::FakeConnectivityTransport`].

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::connectivity::{
    ConnectivityBackend, ConnectivityTransport, HostConnectivity, NetworkDeviceId,
    NetworkProfileId, ProxyBackend, RawActiveConnection, RawDeviceIpFacts, RawNetworkDevice,
    RawNetworkProfile, RawWifiNetwork,
};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::connectivity::parsers::{
    parse_active_connections, parse_active_ssid, parse_connection_show, parse_connectivity,
    parse_device_ip_facts, parse_device_status, parse_radio_state, parse_secret_presence,
    parse_terse_property, parse_wifi_list,
};
use crate::os_control::connectivity::selection::{
    list_active_argv, list_devices_argv, list_profiles_argv, proxy_get_argv,
    query_active_ssid_argv, query_connectivity_argv, query_device_ip_argv,
    query_profile_property_argv, query_radio_argv, scan_wifi_argv,
};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest, TrustedExecutable,
};
use crate::os_control::linux::structured_query::StructuredQueryRequest;
use crate::os_control::receipt::ApplyOutcome;
use crate::os_control::secrets::{SecretPayload, SecretResolutionRequest};

/// The live NetworkManager D-Bus / `nmcli` connectivity adapter. Constructible
/// only in a live composition; a value cannot exist under `os-control-test`.
pub struct LiveNetworkManager {
    backend: ConnectivityBackend,
    _seal: (),
}

impl LiveNetworkManager {
    /// Construct in a live composition root over a selected backend. Requires
    /// a [`LiveHostAccessToken`], so no completion test can build one.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken, backend: ConnectivityBackend) -> Self {
        Self { backend, _seal: () }
    }

    /// The fail-closed placeholder until the desktop root wires the live query /
    /// launch path. Live builds (sentinel disarmed) return `Unavailable` with no
    /// ungoverned fallback; deny-live builds never reach this (the sentinel
    /// aborts first).
    /// Run one governed observation and return its bounded stdout.
    ///
    /// Reads go through [`StructuredQueryRequest`] rather than a bare `Command`,
    /// so an observation inherits the same trusted-executable, exact-argv,
    /// hermetic-environment, output-bound, deadline and cancellation discipline a
    /// mutation gets. A read has no grant to seal against because it changes
    /// nothing.
    ///
    /// `nmcli` is always invoked with `-t` (terse) by the argv builders, so
    /// parsing never depends on column alignment or locale.
    async fn query(
        &self,
        ctx: &HostExecutionContext,
        action: &str,
        argv: Vec<String>,
    ) -> Result<String, OsControlError> {
        self.query_with(ctx, action, self.backend.trusted_executable()?, argv)
            .await
    }

    /// Run one governed observation against an explicit trusted executable.
    ///
    /// The desktop proxy is a GSettings value rather than a NetworkManager
    /// connection property, so its read needs a different trusted executable
    /// while keeping the identical governed-observation discipline.
    async fn query_with(
        &self,
        ctx: &HostExecutionContext,
        action: &str,
        executable: TrustedExecutable,
        argv: Vec<String>,
    ) -> Result<String, OsControlError> {
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action,
            serde_json::Value::Null,
            executable,
            argv,
        );
        let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
        let output = request.run().await?;
        if output.truncated {
            // A truncated listing would silently look like a shorter network or
            // device list, so refuse rather than report a partial world.
            return Err(self.unreadable("output was truncated; refusing a partial read"));
        }
        Ok(output.stdout)
    }

    /// A fail-closed read error. Never substituted for a real value: reporting
    /// "radio off" or "no networks" because `nmcli` could not be parsed would let
    /// a mutation verify against a fact that was never observed.
    fn unreadable(&self, reason: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new(format!("network state {reason}")),
            retryable: true,
        }
    }
}

#[async_trait::async_trait]
impl ConnectivityTransport for LiveNetworkManager {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(format!("connectivity-{}", self.backend.as_str()))
    }

    fn selected_backend(&self) -> ConnectivityBackend {
        self.backend
    }

    async fn read_radio_enabled(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<bool, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let out = self
            .query(ctx, "get_network_state", query_radio_argv())
            .await?;
        // `None` means the output was not recognisable. "Radio off" is NOT a safe
        // default: a later enable would verify as already satisfied.
        parse_radio_state(&out).ok_or_else(|| self.unreadable("radio state could not be parsed"))
    }

    async fn read_active_ssid(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Option<String>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let out = self
            .query(ctx, "get_network_state", query_active_ssid_argv())
            .await?;
        // Here `None` is a real fact — no network is active — because the command
        // succeeded. A *failed* command already returned Err above.
        Ok(parse_active_ssid(&out))
    }

    async fn scan_wifi(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawWifiNetwork>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        // An empty list here means "the scan succeeded and saw nothing", which is
        // a different fact from "the scan failed" — the latter already returned
        // Err from `query`, so the two can never be conflated.
        let out = self.query(ctx, "scan_wifi", scan_wifi_argv()).await?;
        Ok(parse_wifi_list(&out))
    }

    async fn list_devices(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawNetworkDevice>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let out = self
            .query(ctx, "list_network_devices", list_devices_argv())
            .await?;
        Ok(parse_device_status(&out))
    }

    async fn list_profiles(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawNetworkProfile>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let out = self
            .query(ctx, "list_network_profiles", list_profiles_argv())
            .await?;
        Ok(parse_connection_show(&out))
    }

    async fn read_device_connected(
        &self,
        ctx: &HostExecutionContext,
        device: &NetworkDeviceId,
    ) -> Result<bool, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let devices = self.list_devices(ctx).await?;
        // A device that is not in the listing is absent, not disconnected. Those
        // are different facts: reporting "disconnected" for a device that was
        // unplugged would let a disconnect verify against a device that is gone.
        devices
            .iter()
            .find(|candidate| candidate.name == device.as_str())
            .map(RawNetworkDevice::is_connected)
            .ok_or_else(|| self.unreadable("device is not present in the device listing"))
    }

    async fn read_profile_saved(
        &self,
        ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
    ) -> Result<bool, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let profiles = self.list_profiles(ctx).await?;
        // Match on UUID first — a profile NAME is not unique, so matching only by
        // name could report a different profile as saved. The name is accepted as
        // a fallback because callers legitimately hold either identity.
        let id = profile.as_str();
        Ok(profiles
            .iter()
            .any(|candidate| candidate.uuid == id || candidate.name == id))
    }

    async fn read_active_profile(
        &self,
        ctx: &HostExecutionContext,
        device: Option<&NetworkDeviceId>,
    ) -> Result<Option<NetworkProfileId>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let profiles = self.list_profiles(ctx).await?;
        // A profile is active when nmcli bound it to a device. When the caller
        // named a device, only that device's profile counts — otherwise a second
        // interface's connection could be reported as this one's.
        let active = profiles.into_iter().find(|candidate| {
            candidate.is_active()
                && match device {
                    Some(wanted) => candidate.device.as_deref() == Some(wanted.as_str()),
                    None => true,
                }
        });
        // The UUID is the stable identity; the display name is not unique.
        Ok(active.map(|profile| NetworkProfileId::new(profile.uuid)))
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The governed request's own launch trips the deny-live sentinel; keep an
        // explicit guard here too so the adapter is unreachable under test.
        deny_live_transport(RawTransportKind::Process);
        request.dispatch().await
    }

    // ── Tasks 4.2 / 5.3 / 5.6 primitives ────────────────────────────────────

    async fn list_active_connections(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<RawActiveConnection>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        // An empty list means "the query succeeded and nothing is active"; a
        // failed query already returned Err from `query`.
        let out = self
            .query(ctx, "list_active_connections", list_active_argv())
            .await?;
        Ok(parse_active_connections(&out))
    }

    async fn read_profile_property(
        &self,
        ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
        property: &str,
    ) -> Result<Option<String>, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let out = self
            .query(
                ctx,
                "read_network_profile_property",
                query_profile_property_argv(property, profile.as_str()),
            )
            .await?;
        // `None` here is a real fact — the backend did not report the property —
        // because a failed query already returned Err.
        Ok(parse_terse_property(&out, property))
    }

    async fn read_secret_present(
        &self,
        ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
        property: &str,
    ) -> Result<bool, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        // `--show-secrets` is never passed, so the value itself never reaches
        // this process: only the `<hidden>` marker or an empty field.
        let out = self
            .query(
                ctx,
                "read_network_credential_presence",
                query_profile_property_argv(property, profile.as_str()),
            )
            .await?;
        parse_secret_presence(&out, property).ok_or_else(|| {
            self.unreadable("stored credential presence could not be determined")
        })
    }

    async fn read_connectivity(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<HostConnectivity, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let out = self
            .query(ctx, "diagnose_network", query_connectivity_argv())
            .await?;
        // `None` means unrecognised output. NetworkManager's own `unknown`
        // verdict maps to `Undetermined` inside the parser, so "could not
        // determine" is never reported as "no internet".
        parse_connectivity(&out)
            .ok_or_else(|| self.unreadable("connectivity verdict could not be parsed"))
    }

    async fn read_device_ip_facts(
        &self,
        ctx: &HostExecutionContext,
        device: &NetworkDeviceId,
    ) -> Result<RawDeviceIpFacts, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let out = self
            .query(
                ctx,
                "diagnose_network",
                query_device_ip_argv(device.as_str()),
            )
            .await?;
        parse_device_ip_facts(&out)
            .ok_or_else(|| self.unreadable("device IP configuration could not be parsed"))
    }

    async fn read_proxy_key(
        &self,
        ctx: &HostExecutionContext,
        schema: &str,
        key: &str,
    ) -> Result<String, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        // The desktop proxy lives in GSettings, not in a NetworkManager
        // connection property, so this read uses its own trusted executable.
        self.query_with(
            ctx,
            "get_proxy_state",
            self.proxy_backend().trusted_executable()?,
            proxy_get_argv(schema, key),
        )
        .await
    }

    fn proxy_backend(&self) -> ProxyBackend {
        ProxyBackend::GSettings
    }

    async fn resolve_credential(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _request: &SecretResolutionRequest,
    ) -> Result<SecretPayload, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        // Fail closed until a live composition root injects a CredentialStore.
        // Never fall back to an unresolved or empty payload: that would put an
        // unprotected access point on the air, or overwrite a stored credential
        // with nothing.
        Err(OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new(
                "no credential store is composed for this connectivity provider",
            ),
            retryable: false,
        })
    }
}
