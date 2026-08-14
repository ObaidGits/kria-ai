//! Live BlueZ (`org.bluez`) / `bluetoothctl` adapter (raw transport seam).
//!
//! linux-os-control-production **Task 3.7** (OSC-021, OSC-029, OSC-031),
//! design §9.
//!
//! # Host safety
//!
//! Reading adapter/device state or driving a pairing is a **raw live
//! transport**. Like its sibling adapters this one:
//!
//! 1. can be constructed **only** with a
//!    [`crate::os_control::access::LiveHostAccessToken`] (mintable solely in a
//!    live composition root under `os-control-live`), so no completion test can
//!    build it; and
//! 2. calls [`crate::os_control::access::deny_live_transport`] **before** any
//!    read or dispatch, so a deny-live (`os-control-test`) build that reached
//!    here would trip the sentinel and abort rather than touch the radio.
//!
//! # Privacy (OSC-029)
//!
//! A scan enumerates hardware around the user, so discovery is treated as
//! sensitive throughout: results carry redacted labels, addresses are the only
//! identity, and a passkey is never accepted, logged, or persisted.
//!
//! Reads run through [`StructuredQueryRequest`] and mutations through
//! [`StructuredCommandRequest`], so both inherit the same containment. Reads fail
//! closed on unparseable output rather than defaulting: reporting "powered off"
//! because `bluetoothctl` could not be parsed would let an enable request verify
//! as already satisfied. Deny-live tests inject
//! [`crate::os_control::bluetooth::fake::FakeBluetoothTransport`].

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::bluetooth::selection::{
    device_info_argv, list_devices_argv, parse_adapter, parse_device_info, parse_device_list,
    scan_argv, show_adapter_argv,
};
use crate::os_control::bluetooth::{
    BluetoothAdapterId, BluetoothAdapterState, BluetoothBackend, BluetoothDeviceId,
    BluetoothDeviceState, BluetoothScan, BluetoothTransport, DiscoveredDevice,
};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest,
};
use crate::os_control::linux::structured_query::StructuredQueryRequest;
use crate::os_control::receipt::ApplyOutcome;

/// The live BlueZ adapter. Constructible only in a live composition; a value
/// cannot exist under `os-control-test`.
pub struct LiveBluez {
    backend: BluetoothBackend,
    _seal: (),
}

impl LiveBluez {
    /// Construct in a live composition root over a selected backend. Requires a
    /// [`LiveHostAccessToken`], so no completion test can build one.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken, backend: BluetoothBackend) -> Self {
        Self {
            backend,
            _seal: (),
        }
    }

    /// Run one governed observation and return its bounded stdout.
    async fn query(
        &self,
        ctx: &HostExecutionContext,
        action: &str,
        argv: Vec<String>,
    ) -> Result<String, OsControlError> {
        // Reading adapter/device state runs a query child process.
        deny_live_transport(RawTransportKind::Process);
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action,
            serde_json::Value::Null,
            self.backend.trusted_executable()?,
            argv,
        );
        let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
        let output = request.run().await?;
        if output.truncated {
            return Err(OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(
                    "bluetooth state output was truncated; refusing a partial read",
                ),
                retryable: true,
            });
        }
        Ok(output.stdout)
    }
}

#[async_trait::async_trait]
impl BluetoothTransport for LiveBluez {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(format!("bluez-{}", self.backend.as_str()))
    }

    fn selected_backend(&self) -> BluetoothBackend {
        self.backend
    }

    async fn read_adapter(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Option<BluetoothAdapterState>, OsControlError> {
        let out = self
            .query(ctx, "get_bluetooth_state", show_adapter_argv())
            .await?;
        // `None` here means the host positively reports no controller. An
        // unparseable reading is an error, never "powered off" — that default
        // would let an enable request verify as already satisfied.
        Ok(parse_adapter(&out)?.map(|(adapter, powered, discovering)| {
            BluetoothAdapterState {
                adapter: BluetoothAdapterId::new(adapter),
                powered,
                discovering,
            }
        }))
    }

    async fn read_devices(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<BluetoothDeviceState>, OsControlError> {
        let listing = self
            .query(ctx, "get_bluetooth_state", list_devices_argv())
            .await?;
        let mut devices = Vec::new();
        for (address, label) in parse_device_list(&listing)? {
            let info = self
                .query(ctx, "get_bluetooth_state", device_info_argv(&address)?)
                .await?;
            // A device that vanished between the listing and its detail read is
            // skipped rather than reported with guessed flags.
            if let Some((paired, connected, trusted, detail_label)) = parse_device_info(&info)? {
                devices.push(BluetoothDeviceState {
                    device: BluetoothDeviceId::new(address),
                    label: SafeText::new(if detail_label.is_empty() {
                        label
                    } else {
                        detail_label
                    }),
                    paired,
                    connected,
                    trusted,
                    // bluetoothctl reports battery only for some profiles; absent
                    // stays absent rather than becoming a fabricated 0%.
                    battery_percent: None,
                });
            }
        }
        Ok(devices)
    }

    async fn scan(
        &self,
        ctx: &HostExecutionContext,
        duration_ms: u64,
    ) -> Result<BluetoothScan, OsControlError> {
        // The scan itself is bounded by `--timeout`, so discovery can never be
        // left running after the observation returns.
        let _ = self.query(ctx, "scan_bluetooth", scan_argv(duration_ms)).await?;
        // Discovery results are read back from the device listing the scan
        // populated; the scan command's own transcript is progress noise.
        let listing = self
            .query(ctx, "scan_bluetooth", list_devices_argv())
            .await?;
        let devices = parse_device_list(&listing)?
            .into_iter()
            .map(|(address, label)| DiscoveredDevice {
                device: BluetoothDeviceId::new(address),
                label: SafeText::new(label),
                // bluetoothctl's listing carries no RSSI; absent rather than
                // invented.
                rssi: None,
                paired: false,
            })
            .collect();
        Ok(BluetoothScan {
            devices,
            truncated: false,
        })
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
}
