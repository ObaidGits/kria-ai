//! Bluetooth tool handlers (Task 3.7, OSC-021, OSC-029).
//!
//! The eight frozen §10 operations. Every one routes through the governed
//! runtime: reads take an admitted observation context, mutations take a sealed
//! mutation permit. Nothing here touches `bluetoothctl` or the system bus.
//!
//! # Risk shape (from the frozen manifest)
//!
//! * `get_bluetooth_state`, `scan_bluetooth` — **RED reads**. Enumerating nearby
//!   hardware is privacy-sensitive, so they admit as privacy-sensitive and fail
//!   closed when the audit ledger is unhealthy.
//! * `pair_bluetooth_device`, `set_bluetooth_trust`, `remove_bluetooth_device` —
//!   fixed **RED** mutations: each needs a durable human decision, which the
//!   existing policy gate already demands. Pairing therefore rides the existing
//!   approval events with no new frontend authority.
//! * `set_bluetooth_enabled`, `disconnect_bluetooth_device` — **YELLOW**.
//! * `connect_bluetooth_device` — RED for a *new* association, YELLOW for a
//!   device already paired. The handler reports which case applies so the
//!   decision record shows why.
//!
//! A passkey is never accepted as a parameter, so there is nothing to persist.

use std::sync::Arc;

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::os_control::bluetooth::{BluetoothDeviceId, BluetoothOp, BluetoothRequest};
use crate::tools::os_governed as gov;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;

fn device_of(params: &serde_json::Value) -> Option<BluetoothDeviceId> {
    params["device"]
        .as_str()
        .or_else(|| params["address"].as_str())
        .map(BluetoothDeviceId::new)
        .filter(|d| !d.as_str().is_empty())
}

fn missing_device(tool: &str) -> ToolResult {
    ToolResult::err(format!(
        "{tool} requires a `device` address; a device NAME is not an identity because it is neither unique nor stable"
    ))
}

/// Drive one governed Bluetooth mutation.
async fn run_bluetooth(ctx: &ToolContext, tool: &str, op: BluetoothOp) -> ToolResult {
    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.bluetooth(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };

    // Canonical params carry only the address and the boolean intent — never a
    // passkey (OSC-029).
    let params = match &op {
        BluetoothOp::SetEnabled(enabled) => serde_json::json!({ "enabled": enabled }),
        BluetoothOp::SetTrust { device, trusted } => {
            serde_json::json!({ "device": device.as_str(), "trusted": trusted })
        }
        other => serde_json::json!({ "device": other.device().map(|d| d.as_str()) }),
    };

    let request = BluetoothRequest {
        action: tool.to_string(),
        params,
        op,
    };
    let desired = request.desired_state();
    let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);

    gov::run_mutation(
        tool,
        &resolved.runtime,
        provider,
        call,
        &request,
        &desired,
        &plan,
    )
    .await
}

// ── get_bluetooth_state (RED read) ──────────────────────────────────────────

struct GetBluetoothState;

#[async_trait]
impl ToolHandler for GetBluetoothState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_bluetooth_state")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "get_bluetooth_state";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.bluetooth(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.read_state(call.observation()).await {
            Ok(state) => ToolResult::ok(serde_json::json!({
                "backend": provider.backend().as_str(),
                "adapter": state.adapter.as_ref().map(|a| serde_json::json!({
                    "adapter": a.adapter.as_str(),
                    "powered": a.powered,
                    "discovering": a.discovering,
                })),
                "devices": state
                    .devices
                    .iter()
                    .map(|d| serde_json::json!({
                        "device": d.device.as_str(),
                        "label": d.label.as_str(),
                        "paired": d.paired,
                        "connected": d.connected,
                        "trusted": d.trusted,
                        "battery_percent": d.battery_percent,
                    }))
                    .collect::<Vec<_>>(),
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ── scan_bluetooth (RED read) ───────────────────────────────────────────────

struct ScanBluetooth;

#[async_trait]
impl ToolHandler for ScanBluetooth {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "scan_bluetooth")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "scan_bluetooth";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.bluetooth(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        // The provider clamps this to its bounded maximum; a caller cannot request
        // an indefinite radio sweep.
        let duration_ms = params["duration_ms"].as_u64().unwrap_or(5_000);
        match provider.scan(call.observation(), duration_ms).await {
            Ok(scan) => ToolResult::ok(serde_json::json!({
                "devices": scan
                    .devices
                    .iter()
                    .map(|d| serde_json::json!({
                        "device": d.device.as_str(),
                        "label": d.label.as_str(),
                        "rssi": d.rssi,
                        "paired": d.paired,
                    }))
                    .collect::<Vec<_>>(),
                "truncated": scan.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ── mutations ───────────────────────────────────────────────────────────────

struct SetBluetoothEnabled;

#[async_trait]
impl ToolHandler for SetBluetoothEnabled {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_bluetooth_enabled")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let Some(enabled) = params["enabled"].as_bool() else {
            return ToolResult::err("set_bluetooth_enabled requires a boolean `enabled`");
        };
        run_bluetooth(&ctx, "set_bluetooth_enabled", BluetoothOp::SetEnabled(enabled)).await
    }
}

struct PairBluetoothDevice;

#[async_trait]
impl ToolHandler for PairBluetoothDevice {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "pair_bluetooth_device")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "pair_bluetooth_device";
        let Some(device) = device_of(&params) else {
            return missing_device(tool);
        };
        // RED by contract: the gate has already obtained a durable decision, and
        // any agent confirmation rides those existing approval events.
        run_bluetooth(&ctx, tool, BluetoothOp::Pair(device)).await
    }
}

struct ConnectBluetoothDevice;

#[async_trait]
impl ToolHandler for ConnectBluetoothDevice {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "connect_bluetooth_device")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "connect_bluetooth_device";
        let Some(device) = device_of(&params) else {
            return missing_device(tool);
        };
        run_bluetooth(&ctx, tool, BluetoothOp::Connect(device)).await
    }
}

struct DisconnectBluetoothDevice;

#[async_trait]
impl ToolHandler for DisconnectBluetoothDevice {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "disconnect_bluetooth_device")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "disconnect_bluetooth_device";
        let Some(device) = device_of(&params) else {
            return missing_device(tool);
        };
        run_bluetooth(&ctx, tool, BluetoothOp::Disconnect(device)).await
    }
}

struct SetBluetoothTrust;

#[async_trait]
impl ToolHandler for SetBluetoothTrust {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_bluetooth_trust")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "set_bluetooth_trust";
        let Some(device) = device_of(&params) else {
            return missing_device(tool);
        };
        let Some(trusted) = params["trusted"].as_bool() else {
            return ToolResult::err("set_bluetooth_trust requires a boolean `trusted`");
        };
        // RED: trusting a device lets it reconnect without further prompting, so
        // it always requires an explicit decision.
        run_bluetooth(&ctx, tool, BluetoothOp::SetTrust { device, trusted }).await
    }
}

struct RemoveBluetoothDevice;

#[async_trait]
impl ToolHandler for RemoveBluetoothDevice {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "remove_bluetooth_device")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "remove_bluetooth_device";
        let Some(device) = device_of(&params) else {
            return missing_device(tool);
        };
        // RED and irreversible: the pairing keys are destroyed, so the receipt
        // never advertises a rollback.
        run_bluetooth(&ctx, tool, BluetoothOp::Remove(device)).await
    }
}

/// Register the Bluetooth tool surface.
pub fn register(registry: &ToolRegistry) {
    let device = || {
        param(
            "device",
            "string",
            "Bluetooth device address (e.g. AA:BB:CC:DD:EE:FF). A device NAME is not accepted as an identity: it is neither unique nor stable.",
            true,
        )
    };

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "get_bluetooth_state".into(),
                description: "Read the Bluetooth adapter state and known devices".into(),
                category: "bluetooth".into(),
                // RED: enumerating nearby hardware is privacy-sensitive.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetBluetoothState),
        ),
        (
            ToolDef {
                name: "scan_bluetooth".into(),
                description: "Scan for nearby Bluetooth devices (bounded duration)".into(),
                category: "bluetooth".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param(
                    "duration_ms",
                    "integer",
                    "Scan duration in milliseconds (clamped to a bounded maximum)",
                    true,
                )],
            },
            Arc::new(ScanBluetooth),
        ),
        (
            ToolDef {
                name: "set_bluetooth_enabled".into(),
                description: "Power the Bluetooth adapter on or off".into(),
                category: "bluetooth".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param(
                    "enabled",
                    "boolean",
                    "Whether the adapter should be powered on",
                    true,
                )],
            },
            Arc::new(SetBluetoothEnabled),
        ),
        (
            ToolDef {
                name: "pair_bluetooth_device".into(),
                description: "Pair a Bluetooth device (requires explicit approval)".into(),
                category: "bluetooth".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![device()],
            },
            Arc::new(PairBluetoothDevice),
        ),
        (
            ToolDef {
                name: "connect_bluetooth_device".into(),
                description: "Connect a Bluetooth device".into(),
                category: "bluetooth".into(),
                // RED for a new association, YELLOW once paired; the gate
                // re-evaluates per call, so declare the stricter default here.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![device()],
            },
            Arc::new(ConnectBluetoothDevice),
        ),
        (
            ToolDef {
                name: "disconnect_bluetooth_device".into(),
                description: "Disconnect a Bluetooth device".into(),
                category: "bluetooth".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![device()],
            },
            Arc::new(DisconnectBluetoothDevice),
        ),
        (
            ToolDef {
                name: "set_bluetooth_trust".into(),
                description: "Set or clear a Bluetooth device's trust flag (requires approval)"
                    .into(),
                category: "bluetooth".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    device(),
                    param("trusted", "boolean", "Desired trust state", true),
                ],
            },
            Arc::new(SetBluetoothTrust),
        ),
        (
            ToolDef {
                name: "remove_bluetooth_device".into(),
                description: "Remove (unpair and forget) a Bluetooth device — irreversible".into(),
                category: "bluetooth".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![device()],
            },
            Arc::new(RemoveBluetoothDevice),
        ),
    ];

    for (def, handler) in tools {
        registry.register(def, handler);
    }
}

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}
