//! Disk tools: `clean_temp_files` (legacy, RED, out of this spec's OS-control
//! manifest scope per Task 2.5's legacy-difference report) plus the new
//! storage/removable-media lifecycle tools added by
//! linux-os-control-production **Task 3.2** — "Complete storage and
//! removable-media lifecycle" (OSC-012, OSC-030): `list_storage_devices`,
//! `mount_device`, `unmount_device`, `eject_device`, `get_storage_health`.
//!
//! The new tools reach host effects **only** through the injected
//! [`OsControlRuntime`] + `os_control::storage::StorageControl` provider —
//! never a direct `udisksctl`/`mount`/`umount`/`eject` subprocess. Until a
//! live UDisks2 transport is composed into the runtime (desktop startup
//! root), every handler fails closed with the frozen `Unavailable` envelope.
//! There is no `force` parameter and no format/partition/resize/secure-erase/
//! encryption-provisioning tool anywhere in this module (OSC-012.4,
//! OSC-012.6, OSC-030) — that destructive disk administration is handed off
//! to trusted system utilities outside KRIA's tool surface, and remains
//! separately BLACK-blocked at the raw-shell layer
//! (`safety::black_scope`, Task 0.2).

use crate::infra::ToolResult;
use crate::os_control::contract::SafeText;
use crate::os_control::{OsControlError, OsControlRuntime};
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::os_governed as gov;
use crate::tools::ToolContext;
use async_trait::async_trait;
use std::sync::Arc;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

/// Return the governed OS-control `Unavailable` envelope for a storage tool.
///
/// Migrated storage handlers reach host effects **only** through the
/// injected [`OsControlRuntime`] + `os_control::storage::StorageControl`
/// provider — never a direct `udisksctl`/`mount`/`umount`/`eject` subprocess.
/// Until a live UDisks2 transport is composed into the runtime (desktop
/// startup root), the handlers fail closed with this frozen envelope.
fn os_storage_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
    let err = match runtime {
        Some(rt) => rt.unavailable(tool),
        None => OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new("OS control runtime is not injected in this build"),
            retryable: false,
        },
    };
    ToolResult::err_with_data(err.code(), err.to_envelope())
}

struct ListStorageDevices;
#[async_trait]
impl ToolHandler for ListStorageDevices {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_storage_unavailable(None, "list_storage_devices")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // The governed StorageControl provider owns the actual UDisks2
        // object-tree read through the runtime.
        let resolved = match gov::resolve(&ctx, "list_storage_devices") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.storage("list_storage_devices") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "list_storage_devices") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let limit = params["limit"].as_u64().unwrap_or(50).min(200) as usize;
        let cursor = params["cursor"].as_u64().unwrap_or(0) as usize;
        match provider.list_devices(call.observation(), cursor, limit).await {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "devices": page
                    .items
                    .iter()
                    .map(|d| serde_json::json!({
                        "device_id": d.device_id.to_string(),
                        "capacity_bytes": d.capacity_bytes,
                        "free_bytes": d.free_bytes,
                        "mount_point": d.mount_point,
                        "removable": d.removable,
                    }))
                    .collect::<Vec<_>>(),
                "truncated": page.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct MountDevice;
#[async_trait]
impl ToolHandler for MountDevice {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_storage_unavailable(None, "mount_device")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let device = params["device"].as_str().unwrap_or("").trim();
        if device.is_empty() {
            return ToolResult::err("device parameter is required");
        }
        // The governed StorageControl provider owns the actual UDisks2
        // filesystem-mount dispatch (UDisks2's own typed Polkit
        // authorization) + fresh mount-topology verification through the
        // runtime.
        let resolved = match gov::resolve(&ctx, "mount_device") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.storage("mount_device") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "mount_device") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let device_id = params["device_id"].as_str().unwrap_or_default().to_string();
        let request = crate::os_control::storage::StorageRequest {
            action: "mount_device".to_string(),
            params: params.clone(),
            // Never a force flag: unmount/eject refuse to strand in-flight IO
            // (OSC-012.4).
            op: crate::os_control::storage::StorageOp::Mount {
            device: crate::os_control::storage::StorageDeviceId::new(device_id),
                filesystem: params["filesystem_id"]
                    .as_str()
                    .map(crate::os_control::storage::FilesystemId::new),
            },
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "mount_device",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct UnmountDevice;
#[async_trait]
impl ToolHandler for UnmountDevice {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_storage_unavailable(None, "unmount_device")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let device = params["device"].as_str().unwrap_or("").trim();
        if device.is_empty() {
            return ToolResult::err("device parameter is required");
        }
        // The governed StorageControl provider owns the actual UDisks2
        // filesystem-unmount dispatch. A busy device (open file handle)
        // reports a distinct blocking `ResourceBusy` state — there is no
        // force parameter here and never will be (OSC-012.3, OSC-012.4).
        let resolved = match gov::resolve(&ctx, "unmount_device") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.storage("unmount_device") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "unmount_device") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let device_id = params["device_id"].as_str().unwrap_or_default().to_string();
        let request = crate::os_control::storage::StorageRequest {
            action: "unmount_device".to_string(),
            params: params.clone(),
            // Never a force flag: unmount/eject refuse to strand in-flight IO
            // (OSC-012.4).
            op: crate::os_control::storage::StorageOp::Unmount {
            device: crate::os_control::storage::StorageDeviceId::new(device_id),
            },
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "unmount_device",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct EjectDevice;
#[async_trait]
impl ToolHandler for EjectDevice {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_storage_unavailable(None, "eject_device")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let device = params["device"].as_str().unwrap_or("").trim();
        if device.is_empty() {
            return ToolResult::err("device parameter is required");
        }
        // The governed StorageControl provider owns the actual UDisks2
        // eject dispatch. Busy reports `ResourceBusy`; there is no force
        // parameter here and never will be (OSC-012.3, OSC-012.4).
        let resolved = match gov::resolve(&ctx, "eject_device") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.storage("eject_device") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "eject_device") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let device_id = params["device_id"].as_str().unwrap_or_default().to_string();
        let request = crate::os_control::storage::StorageRequest {
            action: "eject_device".to_string(),
            params: params.clone(),
            // Never a force flag: unmount/eject refuse to strand in-flight IO
            // (OSC-012.4).
            op: crate::os_control::storage::StorageOp::Eject {
            device: crate::os_control::storage::StorageDeviceId::new(device_id),
            },
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "eject_device",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct GetStorageHealth;
#[async_trait]
impl ToolHandler for GetStorageHealth {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_storage_unavailable(None, "get_storage_health")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // The governed StorageControl provider owns the actual SMART/health
        // evidence read through the runtime. Missing evidence is reported
        // as a distinct "unavailable" state — never a fabricated
        // healthy/unhealthy status (OSC-012.5, OSC-031).
        let resolved = match gov::resolve(&ctx, "get_storage_health") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.storage("get_storage_health") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "get_storage_health") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let device = params["device_id"]
            .as_str()
            .map(crate::os_control::storage::StorageDeviceId::new);
        match provider.read_health(call.observation(), device.as_ref()).await {
            Ok(report) => ToolResult::ok(serde_json::json!({
                "device_id": report.device_id.to_string(),
                "health_state": report.health_state,
                "temperature_millikelvin": report.temperature_millikelvin,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct CleanTempFiles;
#[async_trait]
impl ToolHandler for CleanTempFiles {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let older_than_days = params["older_than_days"].as_u64().unwrap_or(7);
        let temp_dir = std::env::temp_dir();
        let threshold =
            std::time::SystemTime::now() - std::time::Duration::from_secs(older_than_days * 86400);

        let mut deleted = 0u64;
        let mut freed_bytes = 0u64;

        if let Ok(entries) = std::fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if modified < threshold {
                            freed_bytes += meta.len();
                            if meta.is_dir() {
                                let _ = std::fs::remove_dir_all(entry.path());
                            } else {
                                let _ = std::fs::remove_file(entry.path());
                            }
                            deleted += 1;
                        }
                    }
                }
            }
        }

        ToolResult::ok(serde_json::json!({
            "temp_dir": temp_dir.to_string_lossy(),
            "files_deleted": deleted,
            "freed_mb": freed_bytes / (1024 * 1024),
            "older_than_days": older_than_days,
        }))
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        // RED
        (
            ToolDef {
                name: "clean_temp_files".into(),
                description: "Delete old temporary files".into(),
                category: "disk".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param(
                    "older_than_days",
                    "integer",
                    "Only delete files older than N days (default 7)",
                    false,
                )],
            },
            Arc::new(CleanTempFiles),
        ),
        // GREEN
        (
            ToolDef {
                name: "list_storage_devices".into(),
                description: "List mounted filesystems and removable devices, with capacity, free space, filesystem type, and mount state. Uses stable typed device identifiers, never raw /dev/sdX device-node strings.".into(),
                category: "disk".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("cursor", "string", "Pagination cursor from a previous call", false),
                    param("limit", "integer", "Maximum devices to return per page", false),
                ],
            },
            Arc::new(ListStorageDevices),
        ),
        (
            ToolDef {
                name: "get_storage_health".into(),
                description: "Get available SMART/health evidence for a storage device (or the primary device if none specified). Reports 'unavailable' honestly when no health evidence exists rather than fabricating a healthy/unhealthy status.".into(),
                category: "disk".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param(
                    "device",
                    "string",
                    "Typed storage device identifier (from list_storage_devices)",
                    false,
                )],
            },
            Arc::new(GetStorageHealth),
        ),
        // RED (mount topology + removable-media lifecycle mutations)
        (
            ToolDef {
                name: "mount_device".into(),
                description: "Mount a storage device by its typed device identifier, optionally at a specific filesystem. Uses UDisks2's own typed Polkit authorization — never a raw device command.".into(),
                category: "disk".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    param("device", "string", "Typed storage device identifier (from list_storage_devices)", true),
                    param("filesystem", "string", "Typed filesystem identifier to mount, if the device has more than one", false),
                ],
            },
            Arc::new(MountDevice),
        ),
        (
            ToolDef {
                name: "unmount_device".into(),
                description: "Unmount a storage device by its typed device identifier. A busy device (open file handle) reports a blocking state rather than forcing — there is no force option.".into(),
                category: "disk".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![param(
                    "device",
                    "string",
                    "Typed storage device identifier (from list_storage_devices)",
                    true,
                )],
            },
            Arc::new(UnmountDevice),
        ),
        (
            ToolDef {
                name: "eject_device".into(),
                description: "Eject a removable storage device (unmount + power-down) by its typed device identifier. A busy device reports a blocking state rather than forcing — there is no force option.".into(),
                category: "disk".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![param(
                    "device",
                    "string",
                    "Typed storage device identifier (from list_storage_devices)",
                    true,
                )],
            },
            Arc::new(EjectDevice),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
