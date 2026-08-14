//! Deny-live fake [`ConnectivityTransport`] (OSC-015, OSC-033), Tasks 2.3 / 3.5.
//!
//! Compiled only under `os-control-test`. Every read is served from a scripted
//! in-memory table and `dispatch` records the request instead of running it, so
//! no `nmcli` child process, NetworkManager D-Bus call, or radio state change can
//! occur. Reads can be scripted to fail, so the "ambiguous parse must surface as
//! an error, never a fabricated state" rule is testable.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, Digest, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::StructuredCommandRequest;
use crate::os_control::receipt::{AppliedDispatch, ApplyOutcome};
use crate::os_control::secrets::{SecretPayload, SecretResolutionRequest};

use super::parsers::{
    HostConnectivity, RawActiveConnection, RawDeviceIpFacts, RawNetworkDevice, RawNetworkProfile,
    RawWifiNetwork,
};
use super::selection::{ConnectivityBackend, ProxyBackend};
use super::{ConnectivityTransport, NetworkDeviceId, NetworkProfileId};

/// Provider identity reported by the fake transport.
pub const FAKE_CONNECTIVITY_PROVIDER_ID: &str = "fake-connectivity";

/// A scripted, in-memory connectivity transport.
pub struct FakeConnectivityTransport {
    backend: ConnectivityBackend,
    /// `None` means the radio state was never scripted. An unscripted read is
    /// **unavailable**, never `true`: a fabricated "radio on" would let a
    /// toggle_wifi verify against a fact nobody read.
    radio_enabled: Option<bool>,
    active_ssid: Option<String>,
    active_profile: Option<NetworkProfileId>,
    networks: Vec<RawWifiNetwork>,
    devices: Vec<RawNetworkDevice>,
    profiles: Vec<RawNetworkProfile>,
    /// Scripted active connections (Tasks 4.2 / 5.3).
    active_connections: Vec<RawActiveConnection>,
    /// Scripted `(profile_uuid, property) -> value` rows. An absent key models
    /// "the backend did not report this property".
    profile_properties: HashMap<(String, String), String>,
    /// Scripted `(profile_uuid, property) -> credential present`. An absent key
    /// models "presence could not be determined" and fails closed.
    secret_presence: HashMap<(String, String), bool>,
    /// Scripted host connectivity verdict.
    connectivity: HostConnectivity,
    /// Scripted per-device IP facts.
    device_ip_facts: HashMap<String, RawDeviceIpFacts>,
    /// Scripted `(schema, key) -> raw gsettings reply`.
    proxy_keys: HashMap<(String, String), String>,
    /// Scripted `secret reference -> bytes`, for provider-only resolution.
    credentials: HashMap<String, Vec<u8>>,
    /// When set, every read fails with this error label (fail-closed testing).
    read_failure: Option<String>,
    dispatched: Mutex<Vec<StructuredCommandRequest>>,

    // ── Ordered read queues ──────────────────────────────────────────────────
    // One governed mutation performs SEVERAL reads (pre-observation, under-lease
    // re-observation, pre-apply rollback snapshot, post-apply re-observation, and
    // an independent verification). A lifecycle is only meaningful if those reads
    // can differ, so each queue is consumed one answer per read.
    //
    // A queue takes precedence over the single scripted value above, and only
    // when it was actually pushed — so the older `with_*` single-value style keeps
    // working for tests that perform exactly one read. An **exhausted** queue is
    // an error, never a fallback: a test that scripted fewer reads than the
    // governed path performs should fail loudly rather than silently reuse a
    // stale answer.
    radio_queue: Mutex<VecDeque<bool>>,
    ssid_queue: Mutex<VecDeque<Option<String>>>,
    scan_queue: Mutex<VecDeque<Vec<RawWifiNetwork>>>,
    profiles_queue: Mutex<VecDeque<Vec<RawNetworkProfile>>>,
    devices_queue: Mutex<VecDeque<Vec<RawNetworkDevice>>>,
    /// `Err(label)` scripts a read that could not determine the state — distinct
    /// from `Ok(false)` ("definitely not connected").
    device_connected_queue: Mutex<VecDeque<Result<bool, String>>>,
    profile_saved_queue: Mutex<VecDeque<bool>>,
    active_profile_queue: Mutex<VecDeque<Option<NetworkProfileId>>>,
    /// The outcome `dispatch` returns when scripted.
    dispatch_outcome: Mutex<Option<ApplyOutcome>>,
}

impl FakeConnectivityTransport {
    /// A fake with the radio on, nothing connected, and no scripted rows.
    #[must_use]
    pub fn new(backend: ConnectivityBackend) -> Self {
        Self {
            backend,
            radio_enabled: None,
            active_ssid: None,
            active_profile: None,
            networks: Vec::new(),
            devices: Vec::new(),
            profiles: Vec::new(),
            active_connections: Vec::new(),
            profile_properties: HashMap::new(),
            secret_presence: HashMap::new(),
            connectivity: HostConnectivity::Undetermined,
            device_ip_facts: HashMap::new(),
            proxy_keys: HashMap::new(),
            credentials: HashMap::new(),
            read_failure: None,
            dispatched: Mutex::new(Vec::new()),
            radio_queue: Mutex::new(VecDeque::new()),
            ssid_queue: Mutex::new(VecDeque::new()),
            scan_queue: Mutex::new(VecDeque::new()),
            profiles_queue: Mutex::new(VecDeque::new()),
            devices_queue: Mutex::new(VecDeque::new()),
            device_connected_queue: Mutex::new(VecDeque::new()),
            profile_saved_queue: Mutex::new(VecDeque::new()),
            active_profile_queue: Mutex::new(VecDeque::new()),
            dispatch_outcome: Mutex::new(None),
        }
    }

    /// Append one radio-state answer to the read queue.
    #[must_use]
    pub fn radio_ok(self, enabled: bool) -> Self {
        self.radio_queue.lock().expect("radio queue").push_back(enabled);
        self
    }

    /// Append one active-SSID answer. `None` is the positive fact "nothing is
    /// connected", not a failed read.
    #[must_use]
    pub fn ssid_ok(self, ssid: Option<&str>) -> Self {
        self.ssid_queue
            .lock()
            .expect("ssid queue")
            .push_back(ssid.map(str::to_string));
        self
    }

    /// Append one scan result. An empty vec means "the scan succeeded and saw
    /// nothing" — a different fact from a failed scan.
    #[must_use]
    pub fn scan_ok(self, networks: Vec<RawWifiNetwork>) -> Self {
        self.scan_queue.lock().expect("scan queue").push_back(networks);
        self
    }

    /// Append one profile listing.
    #[must_use]
    pub fn profiles_ok(self, profiles: Vec<RawNetworkProfile>) -> Self {
        self.profiles_queue
            .lock()
            .expect("profiles queue")
            .push_back(profiles);
        self
    }

    /// Append one device listing.
    #[must_use]
    pub fn devices_ok(self, devices: Vec<RawNetworkDevice>) -> Self {
        self.devices_queue
            .lock()
            .expect("devices queue")
            .push_back(devices);
        self
    }

    /// Append one device-connected answer.
    #[must_use]
    pub fn device_connected_ok(self, connected: bool) -> Self {
        self.device_connected_queue
            .lock()
            .expect("device queue")
            .push_back(Ok(connected));
        self
    }

    /// Append a device-connected read that could NOT determine the state.
    ///
    /// Distinct from `device_connected_ok(false)` on purpose: "not connected" and
    /// "unknown" must never be conflated, and this is how a suite proves it.
    #[must_use]
    pub fn device_connected_err(self) -> Self {
        self.device_connected_queue
            .lock()
            .expect("device queue")
            .push_back(Err("device state could not be determined".to_string()));
        self
    }

    /// Append one profile-saved answer.
    #[must_use]
    pub fn profile_saved_ok(self, saved: bool) -> Self {
        self.profile_saved_queue
            .lock()
            .expect("profile saved queue")
            .push_back(saved);
        self
    }

    /// Append one active-profile answer.
    #[must_use]
    pub fn active_profile_ok(self, profile: Option<NetworkProfileId>) -> Self {
        self.active_profile_queue
            .lock()
            .expect("active profile queue")
            .push_back(profile);
        self
    }

    /// Script the outcome `dispatch` returns.
    #[must_use]
    pub fn dispatch_outcome(self, outcome: ApplyOutcome) -> Self {
        *self.dispatch_outcome.lock().expect("dispatch outcome") = Some(outcome);
        self
    }

    /// An exhausted queue is unknown state, never a reused stale answer.
    fn queue_exhausted(&self, what: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(format!("fake-connectivity-{}", self.backend.as_str()))),
            reason: SafeText::new(format!(
                "scripted read queue for {what} is exhausted; the state is unknown, not a default"
            )),
            retryable: true,
        }
    }

    /// The **redacted** projections of every dispatched command, in order.
    ///
    /// Redacted rather than raw on purpose: a connectivity command can carry a
    /// Wi-Fi or VPN password, so a suite asserts on exactly what an audit record
    /// would show — which also proves the secret never reaches it.
    #[must_use]
    pub fn captured(&self) -> Vec<crate::os_control::linux::structured_command::StructuredCommandSummary> {
        self.dispatched
            .lock()
            .expect("dispatch mutex")
            .iter()
            .map(StructuredCommandRequest::safe_summary)
            .collect()
    }

    /// Builder: select the backend the fake reports.
    #[must_use]
    pub fn with_backend(mut self, backend: ConnectivityBackend) -> Self {
        self.backend = backend;
        self
    }

    /// Builder: set the radio state.
    #[must_use]
    pub fn with_radio_enabled(mut self, enabled: bool) -> Self {
        self.radio_enabled = Some(enabled);
        self
    }

    /// Builder: set the currently active SSID.
    #[must_use]
    pub fn with_active_ssid(mut self, ssid: impl Into<String>) -> Self {
        self.active_ssid = Some(ssid.into());
        self
    }

    /// Builder: set the currently active profile identity.
    #[must_use]
    pub fn with_active_profile(mut self, profile: NetworkProfileId) -> Self {
        self.active_profile = Some(profile);
        self
    }

    /// Builder: script the Wi-Fi scan rows.
    #[must_use]
    pub fn with_networks(mut self, networks: Vec<RawWifiNetwork>) -> Self {
        self.networks = networks;
        self
    }

    /// Builder: script the device rows.
    #[must_use]
    pub fn with_devices(mut self, devices: Vec<RawNetworkDevice>) -> Self {
        self.devices = devices;
        self
    }

    /// Builder: script the saved-profile rows.
    #[must_use]
    pub fn with_profiles(mut self, profiles: Vec<RawNetworkProfile>) -> Self {
        self.profiles = profiles;
        self
    }

    /// Builder: make every read fail, proving a read error is never turned into
    /// a fabricated state.
    #[must_use]
    pub fn with_read_failure(mut self, reason: impl Into<String>) -> Self {
        self.read_failure = Some(reason.into());
        self
    }

    /// Builder: script the active-connection rows (Tasks 4.2 / 5.3).
    #[must_use]
    pub fn with_active_connections(mut self, rows: Vec<RawActiveConnection>) -> Self {
        self.active_connections = rows;
        self
    }

    /// Builder: script one saved-profile property value.
    #[must_use]
    pub fn with_profile_property(
        mut self,
        profile: &str,
        property: &str,
        value: impl Into<String>,
    ) -> Self {
        self.profile_properties
            .insert((profile.to_string(), property.to_string()), value.into());
        self
    }

    /// Builder: script whether a credential is stored for a profile property.
    /// Leaving it unscripted models "presence could not be determined".
    #[must_use]
    pub fn with_secret_presence(mut self, profile: &str, property: &str, present: bool) -> Self {
        self.secret_presence
            .insert((profile.to_string(), property.to_string()), present);
        self
    }

    /// Builder: script the host connectivity verdict.
    #[must_use]
    pub fn with_connectivity(mut self, connectivity: HostConnectivity) -> Self {
        self.connectivity = connectivity;
        self
    }

    /// Builder: script one device's IP facts.
    #[must_use]
    pub fn with_device_ip_facts(mut self, device: &str, facts: RawDeviceIpFacts) -> Self {
        self.device_ip_facts.insert(device.to_string(), facts);
        self
    }

    /// Builder: script one desktop proxy key's raw reply.
    #[must_use]
    pub fn with_proxy_key(mut self, schema: &str, key: &str, value: impl Into<String>) -> Self {
        self.proxy_keys
            .insert((schema.to_string(), key.to_string()), value.into());
        self
    }

    /// Builder: script a resolvable credential. The bytes never leave the fake
    /// except as a [`SecretPayload`], which cannot serialize or display.
    #[must_use]
    pub fn with_credential(mut self, reference: &str, bytes: impl Into<Vec<u8>>) -> Self {
        self.credentials.insert(reference.to_string(), bytes.into());
        self
    }

    /// The structured-command requests this fake was asked to dispatch, in order.
    #[must_use]
    pub fn dispatched(&self) -> Vec<StructuredCommandRequest> {
        self.dispatched.lock().expect("dispatch mutex").clone()
    }

    /// How many dispatches were requested.
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        self.dispatched.lock().expect("dispatch mutex").len()
    }

    fn guard_reads(&self) -> Result<(), OsControlError> {
        match &self.read_failure {
            None => Ok(()),
            Some(reason) => Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(FAKE_CONNECTIVITY_PROVIDER_ID)),
                reason: crate::os_control::contract::SafeText::new(reason.clone()),
                retryable: true,
            }),
        }
    }
}

impl Default for FakeConnectivityTransport {
    fn default() -> Self {
        // `nmcli` is the preferred backend, so it is the honest default.
        Self::new(ConnectivityBackend::Nmcli)
    }
}

#[async_trait]
impl ConnectivityTransport for FakeConnectivityTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_CONNECTIVITY_PROVIDER_ID)
    }

    fn selected_backend(&self) -> ConnectivityBackend {
        self.backend
    }

    async fn read_radio_enabled(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<bool, OsControlError> {
        self.guard_reads()?;
        let mut queue = self.radio_queue.lock().expect("radio queue");
        if queue.is_empty() {
            // Never scripted as a queue: fall back to the single-value style, and
            // if that was never set either, the state is UNKNOWN.
            return self
                .radio_enabled
                .ok_or_else(|| self.queue_exhausted("radio state"));
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("radio state"))
    }

    async fn read_active_ssid(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Option<String>, OsControlError> {
        self.guard_reads()?;
        let mut queue = self.ssid_queue.lock().expect("ssid queue");
        if queue.is_empty() {
            return Ok(self.active_ssid.clone());
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("active ssid"))
    }

    async fn scan_wifi(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Vec<RawWifiNetwork>, OsControlError> {
        self.guard_reads()?;
        let mut queue = self.scan_queue.lock().expect("scan queue");
        if queue.is_empty() {
            return Ok(self.networks.clone());
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("wifi scan"))
    }

    async fn list_devices(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Vec<RawNetworkDevice>, OsControlError> {
        self.guard_reads()?;
        let mut queue = self.devices_queue.lock().expect("devices queue");
        if queue.is_empty() {
            return Ok(self.devices.clone());
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("device listing"))
    }

    async fn list_profiles(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Vec<RawNetworkProfile>, OsControlError> {
        self.guard_reads()?;
        let mut queue = self.profiles_queue.lock().expect("profiles queue");
        if queue.is_empty() {
            return Ok(self.profiles.clone());
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("profile listing"))
    }

    async fn read_device_connected(
        &self,
        _ctx: &HostExecutionContext,
        device: &NetworkDeviceId,
    ) -> Result<bool, OsControlError> {
        self.guard_reads()?;
        let mut queue = self.device_connected_queue.lock().expect("device queue");
        if queue.is_empty() {
            return Ok(self
                .devices
                .iter()
                .any(|d| d.name == device.as_str() && d.is_connected()));
        }
        match queue.pop_front() {
            Some(Ok(connected)) => Ok(connected),
            // An UNKNOWN device state, deliberately distinct from `false`.
            Some(Err(_label)) => Err(self.queue_exhausted("device connected state")),
            None => Err(self.queue_exhausted("device connected state")),
        }
    }

    async fn read_profile_saved(
        &self,
        _ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
    ) -> Result<bool, OsControlError> {
        self.guard_reads()?;
        let mut queue = self.profile_saved_queue.lock().expect("profile saved queue");
        if queue.is_empty() {
            return Ok(self
                .profiles
                .iter()
                .any(|p| p.uuid == profile.as_str() || p.name == profile.as_str()));
        }
        queue
            .pop_front()
            .ok_or_else(|| self.queue_exhausted("profile saved state"))
    }

    async fn read_active_profile(
        &self,
        _ctx: &HostExecutionContext,
        device: Option<&NetworkDeviceId>,
    ) -> Result<Option<NetworkProfileId>, OsControlError> {
        self.guard_reads()?;
        {
            let mut queue = self.active_profile_queue.lock().expect("active profile queue");
            if !queue.is_empty() {
                return queue
                    .pop_front()
                    .ok_or_else(|| self.queue_exhausted("active profile"));
            }
        }
        match device {
            // Device-scoped: only a profile bound to that device counts.
            Some(dev) => Ok(self
                .profiles
                .iter()
                .find(|p| p.device.as_deref() == Some(dev.as_str()))
                .map(|p| NetworkProfileId::new(&p.uuid))),
            None => Ok(self.active_profile.clone()),
        }
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Recorded, never executed: no child process is spawned.
        self.dispatched
            .lock()
            .expect("dispatch mutex")
            .push(request.clone());
        if let Some(outcome) = self.dispatch_outcome.lock().expect("dispatch outcome").clone() {
            return Ok(outcome);
        }
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            Some(Digest::of_str(crate::os_control::testing::FAKE_RECEIPT_TAG)),
            BoundedVec::new(),
        )))
    }

    // ── Tasks 4.2 / 5.3 / 5.6 primitives ────────────────────────────────────

    async fn list_active_connections(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Vec<RawActiveConnection>, OsControlError> {
        self.guard_reads()?;
        Ok(self.active_connections.clone())
    }

    async fn read_profile_property(
        &self,
        _ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
        property: &str,
    ) -> Result<Option<String>, OsControlError> {
        self.guard_reads()?;
        Ok(self
            .profile_properties
            .get(&(profile.as_str().to_string(), property.to_string()))
            .cloned())
    }

    async fn read_secret_present(
        &self,
        _ctx: &HostExecutionContext,
        profile: &NetworkProfileId,
        property: &str,
    ) -> Result<bool, OsControlError> {
        self.guard_reads()?;
        let key = (profile.as_str().to_string(), property.to_string());
        match self.secret_presence.get(&key) {
            Some(present) => Ok(*present),
            // Presence was never scripted: that is "could not determine", which
            // must be an error rather than a fabricated `false`.
            None => Err(OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: crate::os_control::contract::SafeText::new(
                    "stored credential presence could not be determined",
                ),
                retryable: true,
            }),
        }
    }

    async fn read_connectivity(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<HostConnectivity, OsControlError> {
        self.guard_reads()?;
        Ok(self.connectivity)
    }

    async fn read_device_ip_facts(
        &self,
        _ctx: &HostExecutionContext,
        device: &NetworkDeviceId,
    ) -> Result<RawDeviceIpFacts, OsControlError> {
        self.guard_reads()?;
        self.device_ip_facts
            .get(device.as_str())
            .cloned()
            .ok_or_else(|| OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: crate::os_control::contract::SafeText::new(
                    "device IP configuration could not be parsed",
                ),
                retryable: true,
            })
    }

    async fn read_proxy_key(
        &self,
        _ctx: &HostExecutionContext,
        schema: &str,
        key: &str,
    ) -> Result<String, OsControlError> {
        self.guard_reads()?;
        self.proxy_keys
            .get(&(schema.to_string(), key.to_string()))
            .cloned()
            .ok_or_else(|| OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: crate::os_control::contract::SafeText::new(
                    "desktop proxy key could not be read",
                ),
                retryable: true,
            })
    }

    fn proxy_backend(&self) -> ProxyBackend {
        ProxyBackend::GSettings
    }

    async fn resolve_credential(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &SecretResolutionRequest,
    ) -> Result<SecretPayload, OsControlError> {
        match self.credentials.get(request.reference.as_str()) {
            // The scripted bytes are returned as a `SecretPayload`, which cannot
            // serialize or display, so a test cannot accidentally assert on the
            // value or print it.
            Some(bytes) => Ok(SecretPayload::new(bytes.clone())),
            None => Err(crate::os_control::secrets::unknown_reference()),
        }
    }
}
