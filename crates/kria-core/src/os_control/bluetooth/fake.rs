//! Deny-live fake [`BluetoothTransport`] (OSC-021, OSC-033), Task 3.7.
//!
//! Compiled only under `os-control-test`. It models a BlueZ object manager as a
//! plain in-memory adapter plus device table: no system bus, no `bluetoothctl`
//! child process, no radio. `dispatch` records the governed command and applies
//! its effect to the table, so a test can drive a full pair → connect → trust →
//! remove lifecycle and verify each postcondition.
//!
//! Scriptable failure modes exist for the races Task 3.7 names: a device that
//! disappears mid-flight, a scan that times out, and duplicate advertised names.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, Digest, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::StructuredCommandRequest;
use crate::os_control::receipt::{AppliedDispatch, ApplyOutcome};

use super::selection::BluetoothBackend;
use super::{
    BluetoothAdapterId, BluetoothAdapterState, BluetoothDeviceId, BluetoothDeviceState,
    BluetoothScan, BluetoothTransport, DiscoveredDevice,
};

/// Provider identity reported by the fake transport.
pub const FAKE_BLUETOOTH_PROVIDER_ID: &str = "fake-bluetooth";

/// How a scripted read should fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeBluetoothFault {
    /// The adapter is absent entirely (no Bluetooth hardware).
    NoAdapter,
    /// Reads time out (a wedged bus).
    ReadTimeout,
    /// A scan exceeded its deadline.
    ScanTimeout,
}

/// A scripted, in-memory BlueZ object manager.
pub struct FakeBluetoothTransport {
    backend: BluetoothBackend,
    adapter: Mutex<Option<BluetoothAdapterState>>,
    devices: Mutex<HashMap<BluetoothDeviceId, BluetoothDeviceState>>,
    discoverable: Mutex<Vec<DiscoveredDevice>>,
    fault: Mutex<Option<FakeBluetoothFault>>,
    /// Devices that vanish the next time they are read — models a device that
    /// walks out of range mid-operation.
    vanish_on_read: Mutex<Vec<BluetoothDeviceId>>,
    dispatched: Mutex<Vec<StructuredCommandRequest>>,
    scans: Mutex<Vec<u64>>,
}

impl FakeBluetoothTransport {
    /// A fake with a powered adapter and no known devices.
    #[must_use]
    pub fn new(backend: BluetoothBackend) -> Self {
        Self {
            backend,
            adapter: Mutex::new(Some(BluetoothAdapterState {
                adapter: BluetoothAdapterId::new("00:11:22:33:44:55"),
                powered: true,
                discovering: false,
            })),
            devices: Mutex::new(HashMap::new()),
            discoverable: Mutex::new(Vec::new()),
            fault: Mutex::new(None),
            vanish_on_read: Mutex::new(Vec::new()),
            dispatched: Mutex::new(Vec::new()),
            scans: Mutex::new(Vec::new()),
        }
    }

    /// Builder: set the adapter's powered state.
    #[must_use]
    pub fn with_powered(self, powered: bool) -> Self {
        if let Some(adapter) = self.adapter.lock().expect("adapter mutex").as_mut() {
            adapter.powered = powered;
        }
        self
    }

    /// Builder: remove the adapter entirely (no Bluetooth hardware).
    #[must_use]
    pub fn without_adapter(self) -> Self {
        *self.adapter.lock().expect("adapter mutex") = None;
        self
    }

    /// Builder: seed a known device.
    #[must_use]
    pub fn with_device(
        self,
        address: &str,
        label: &str,
        paired: bool,
        connected: bool,
        trusted: bool,
    ) -> Self {
        let id = BluetoothDeviceId::new(address);
        self.devices.lock().expect("devices mutex").insert(
            id.clone(),
            BluetoothDeviceState {
                device: id,
                label: SafeText::new(label),
                paired,
                connected,
                trusted,
                battery_percent: None,
            },
        );
        self
    }

    /// Builder: seed a device that reports a battery level (OSC-021 metadata).
    #[must_use]
    pub fn with_device_battery(self, address: &str, percent: u8) -> Self {
        if let Some(state) = self
            .devices
            .lock()
            .expect("devices mutex")
            .get_mut(&BluetoothDeviceId::new(address))
        {
            state.battery_percent = Some(percent);
        }
        self
    }

    /// Builder: seed a discoverable (not yet known) device for scans.
    #[must_use]
    pub fn with_discoverable(self, address: &str, label: &str, rssi: Option<i16>) -> Self {
        self.discoverable
            .lock()
            .expect("discoverable mutex")
            .push(DiscoveredDevice {
                device: BluetoothDeviceId::new(address),
                label: SafeText::new(label),
                rssi,
                paired: false,
            });
        self
    }

    /// Builder: script a read/scan fault.
    #[must_use]
    pub fn with_fault(self, fault: FakeBluetoothFault) -> Self {
        *self.fault.lock().expect("fault mutex") = Some(fault);
        self
    }

    /// Builder: make `address` disappear the next time devices are read.
    #[must_use]
    pub fn vanishing(self, address: &str) -> Self {
        self.vanish_on_read
            .lock()
            .expect("vanish mutex")
            .push(BluetoothDeviceId::new(address));
        self
    }

    /// The governed commands this fake captured instead of executing, in order.
    #[must_use]
    pub fn captured(&self) -> Vec<StructuredCommandRequest> {
        self.dispatched.lock().expect("dispatch mutex").clone()
    }

    /// How many dispatches were requested.
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        self.dispatched.lock().expect("dispatch mutex").len()
    }

    /// The scan durations requested, in order (proves clamping).
    #[must_use]
    pub fn scan_durations(&self) -> Vec<u64> {
        self.scans.lock().expect("scans mutex").clone()
    }

    /// The current known-device count.
    #[must_use]
    pub fn device_count(&self) -> usize {
        self.devices.lock().expect("devices mutex").len()
    }

    fn guard(&self, scanning: bool) -> Result<(), OsControlError> {
        match *self.fault.lock().expect("fault mutex") {
            None => Ok(()),
            Some(FakeBluetoothFault::NoAdapter) => Ok(()), // surfaced by read_adapter
            Some(FakeBluetoothFault::ReadTimeout) if !scanning => {
                Err(OsControlError::TimedOutBeforeMutation {
                    operation: crate::os_control::contract::SafeOperation::new(
                        "bluetooth.read",
                    ),
                    timeout_ms: 1_500,
                })
            }
            Some(FakeBluetoothFault::ScanTimeout) if scanning => {
                Err(OsControlError::TimedOutBeforeMutation {
                    operation: crate::os_control::contract::SafeOperation::new(
                        "bluetooth.read",
                    ),
                    timeout_ms: 1_500,
                })
            }
            Some(_) => Ok(()),
        }
    }

    /// Apply a captured command's effect to the in-memory table.
    ///
    /// This is what makes the fake useful for lifecycle tests: after `pair`, a
    /// subsequent read genuinely reports the device as paired, so verification
    /// exercises the real comparison rather than a hardcoded answer.
    fn apply_effect(&self, argv: &[String]) {
        let Some(verb) = argv.first().map(String::as_str) else {
            return;
        };
        let target = argv.get(1).map(|a| BluetoothDeviceId::new(a));

        match verb {
            "power" => {
                let on = argv.get(1).map(String::as_str) == Some("on");
                if let Some(adapter) = self.adapter.lock().expect("adapter mutex").as_mut() {
                    adapter.powered = on;
                }
            }
            "pair" | "connect" | "trust" | "untrust" | "disconnect" => {
                let Some(id) = target else { return };
                let mut devices = self.devices.lock().expect("devices mutex");
                if verb == "pair" {
                    // Pairing is the only verb that may create a known device, and
                    // BlueZ connects as part of pairing.
                    let entry = devices.entry(id.clone()).or_insert(BluetoothDeviceState {
                        device: id,
                        label: SafeText::new("scripted device"),
                        paired: false,
                        connected: false,
                        trusted: false,
                        battery_percent: None,
                    });
                    entry.paired = true;
                    entry.connected = true;
                    return;
                }
                // Every other verb acts on an ALREADY-KNOWN device. If it is gone
                // (out of range, removed), the command is a no-op — it must never
                // resurrect the device, or a vanished device would verify as
                // connected.
                let Some(entry) = devices.get_mut(&id) else {
                    return;
                };
                match verb {
                    "connect" => entry.connected = true,
                    "disconnect" => entry.connected = false,
                    "trust" => entry.trusted = true,
                    "untrust" => entry.trusted = false,
                    _ => {}
                }
            }
            "remove" => {
                if let Some(id) = target {
                    self.devices.lock().expect("devices mutex").remove(&id);
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl BluetoothTransport for FakeBluetoothTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_BLUETOOTH_PROVIDER_ID)
    }

    fn selected_backend(&self) -> BluetoothBackend {
        self.backend
    }

    async fn read_adapter(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Option<BluetoothAdapterState>, OsControlError> {
        self.guard(false)?;
        if *self.fault.lock().expect("fault mutex") == Some(FakeBluetoothFault::NoAdapter) {
            return Ok(None);
        }
        Ok(self.adapter.lock().expect("adapter mutex").clone())
    }

    async fn read_devices(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Vec<BluetoothDeviceState>, OsControlError> {
        self.guard(false)?;
        // Honour any scripted disappearance exactly once, modelling a device that
        // leaves range between two reads.
        let vanishing: Vec<BluetoothDeviceId> =
            self.vanish_on_read.lock().expect("vanish mutex").drain(..).collect();
        let mut devices = self.devices.lock().expect("devices mutex");
        for id in vanishing {
            devices.remove(&id);
        }
        Ok(devices.values().cloned().collect())
    }

    async fn scan(
        &self,
        _ctx: &HostExecutionContext,
        duration_ms: u64,
    ) -> Result<BluetoothScan, OsControlError> {
        self.scans.lock().expect("scans mutex").push(duration_ms);
        self.guard(true)?;
        Ok(BluetoothScan {
            devices: self.discoverable.lock().expect("discoverable mutex").clone(),
            truncated: false,
        })
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
        self.apply_effect(request.args());
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            Some(Digest::of_str(crate::os_control::testing::FAKE_RECEIPT_TAG)),
            BoundedVec::new(),
        )))
    }
}
