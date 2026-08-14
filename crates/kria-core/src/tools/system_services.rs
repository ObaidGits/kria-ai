//! Search, system-health, backup, scan, firmware and sensor handlers.
//!
//! linux-os-control-production tasks **4.1**, **4.4**, **4.6**, **5.4**, **5.5**.
//!
//! Every handler routes through [`crate::tools::os_governed`].
//!
//! # The three answers this file is careful never to fake
//!
//! * **"Healthy"** when a check could not run. `diagnose_system` reports
//!   `undetermined` per subsystem and overall, because a false all-clear stops the
//!   user looking for the real fault.
//! * **"Up to date"** when no update source was reachable. `get_firmware_status`
//!   distinguishes "no updates" from "could not check".
//! * **"Backed up"** when a job merely started. `start_backup` verifies acceptance
//!   only — a false assurance about backups is the worst kind.

use std::sync::Arc;

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::os_control::backup::{
    BackupProviderId, BackupSnapshotId, BoundedDpi, JobOp, JobRequest, ScanFormat, ScannerId,
};
use crate::os_control::health::{
    HealthDomain, HealthRequest, LogQuery, RecoveryRecipeId,
};
use crate::os_control::search::{ScopeChange, SearchOp, SearchRequest, SearchScopeId};
use crate::safety::RiskLevel;
use crate::tools::os_governed as gov;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

/// Collect an array of absolute paths, refusing traversal.
fn path_array(params: &serde_json::Value, field: &str) -> Result<Vec<std::path::PathBuf>, ToolResult> {
    let Some(rows) = params[field].as_array() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(raw) = row.as_str() else {
            return Err(ToolResult::err(format!("`{field}` must contain strings")));
        };
        out.push(std::path::PathBuf::from(raw.trim()));
    }
    Ok(out)
}

fn required_path(params: &serde_json::Value, field: &str) -> Result<std::path::PathBuf, ToolResult> {
    let raw = params[field].as_str().unwrap_or("").trim();
    if raw.is_empty() {
        return Err(ToolResult::err(format!("`{field}` is required")));
    }
    let path = std::path::PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(ToolResult::err(format!("`{field}` must be absolute")));
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ToolResult::err(format!("`{field}` must not contain `..`")));
    }
    Ok(path)
}

// ─────────────────────────────────────────────────────────────────────────────
// Search (Task 4.1)
// ─────────────────────────────────────────────────────────────────────────────

async fn run_search(
    ctx: &ToolContext,
    tool: &str,
    params: serde_json::Value,
    op: SearchOp,
) -> ToolResult {
    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.search_control(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };
    let request = SearchRequest {
        action: tool.to_string(),
        params,
        op,
    };
    let observed = match crate::os_control::contract::DesiredStateControl::observe(
        provider,
        call.observation(),
        &request,
    )
    .await
    {
        Ok(observed) => observed,
        Err(error) => return gov::os_error(&error),
    };
    let desired = request.desired_state(&observed);
    let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
    gov::run_mutation(tool, &resolved.runtime, provider, call, &request, &desired, &plan).await
}

struct SearchDesktop;

#[async_trait]
impl ToolHandler for SearchDesktop {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "search_desktop")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "search_desktop";
        let Some(query) = params["query"].as_str().map(str::trim) else {
            return ToolResult::err("`query` is required");
        };
        let scope = match params["scope"].as_str() {
            Some(raw) => match SearchScopeId::parse(raw) {
                Ok(id) => Some(id),
                Err(error) => return gov::os_error(&error),
            },
            None => None,
        };
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.search_control(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let limit = params["limit"].as_u64().and_then(|v| usize::try_from(v).ok());
        match provider
            .search(
                call.observation(),
                query,
                scope.as_ref(),
                params["cursor"].as_str(),
                limit,
            )
            .await
        {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "results": page.items.iter().map(|h| serde_json::json!({
                    "path": h.path.to_string_lossy(),
                    "kind": h.kind,
                    // Present only for a content-indexed scope; a name-only search
                    // has no content to quote.
                    "snippet": h.snippet.as_ref().map(|s| s.as_str()),
                })).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor.as_deref(),
                "truncated": page.truncated,
                // Surfaced so a caller knows whether file CONTENTS were read.
                "content_indexed": page.content_indexed,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct GetSearchScope;

#[async_trait]
impl ToolHandler for GetSearchScope {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_search_scope")
    }

    async fn execute_with_context(&self, _p: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "get_search_scope";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.search_control(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.scope(call.observation(), None).await {
            Ok(scope) => ToolResult::ok(serde_json::json!({
                "scope": scope.scope.as_str(),
                "roots": scope.roots.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
                "exclusions": scope.exclusions.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
                // The flag that decides how sensitive a search of this scope is.
                "content_indexed": scope.content_indexed,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct ConfigureSearchScope;

#[async_trait]
impl ToolHandler for ConfigureSearchScope {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "configure_search_scope")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "configure_search_scope";
        let roots = match path_array(&params, "roots") {
            Ok(roots) => roots,
            Err(result) => return result,
        };
        let exclusions = match path_array(&params, "exclusions") {
            Ok(exclusions) => exclusions,
            Err(result) => return result,
        };
        // Adding a root silently widens what every future search can reach,
        // including file contents — hence the validation and the RED rating.
        let change = match ScopeChange::parse(roots, exclusions) {
            Ok(change) => change,
            Err(error) => return gov::os_error(&error),
        };
        run_search(&ctx, tool, params, SearchOp::ConfigureScope(change)).await
    }
}

struct RebuildSearchIndex;

#[async_trait]
impl ToolHandler for RebuildSearchIndex {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "rebuild_search_index")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "rebuild_search_index";
        let scope = match params["scope"].as_str() {
            Some(raw) => match SearchScopeId::parse(raw) {
                Ok(id) => Some(id),
                Err(error) => return gov::os_error(&error),
            },
            None => None,
        };
        // Verified as accepted-and-running; a full rebuild can take hours.
        run_search(&ctx, tool, params, SearchOp::Rebuild { scope }).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// System health (Task 4.6)
// ─────────────────────────────────────────────────────────────────────────────

struct DiagnoseSystem;

#[async_trait]
impl ToolHandler for DiagnoseSystem {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "diagnose_system")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "diagnose_system";
        let scope = match params["scope"].as_str() {
            Some(raw) => match HealthDomain::parse(raw) {
                Ok(domain) => Some(domain),
                Err(error) => return gov::os_error(&error),
            },
            None => None,
        };
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.health(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.diagnose(call.observation(), scope).await {
            Ok(report) => ToolResult::ok(serde_json::json!({
                // Three-valued: `undetermined` is never rounded to healthy.
                "overall": report.overall().tag(),
                "findings": report.findings.iter().map(|f| serde_json::json!({
                    "subsystem": f.domain.tag(),
                    "verdict": f.verdict.tag(),
                    "detail": f.detail.as_ref().map(|d| d.as_str()),
                })).collect::<Vec<_>>(),
                "note": "`undetermined` means the check could not run — it is not a pass",
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct GetSystemLogs;

#[async_trait]
impl ToolHandler for GetSystemLogs {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_system_logs")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "get_system_logs";
        // Scoped and bounded by construction — there is no "everything" form,
        // because the journal carries auth failures and other users' activity.
        let query = match LogQuery::parse(
            params["query"]["unit"].as_str(),
            u32::try_from(params["query"]["since_hours"].as_u64().unwrap_or(1)).unwrap_or(1),
            u32::try_from(params["query"]["max_lines"].as_u64().unwrap_or(100)).unwrap_or(100),
            u8::try_from(params["query"]["max_priority"].as_u64().unwrap_or(6)).unwrap_or(6),
        ) {
            Ok(query) => query,
            Err(error) => return gov::os_error(&error),
        };
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.health(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.logs(call.observation(), &query).await {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "lines": page.lines.iter().map(|l| serde_json::json!({
                    "timestamp": l.timestamp,
                    "unit": l.unit,
                    "priority": l.priority,
                    "message": l.message.as_str(),
                })).collect::<Vec<_>>(),
                "truncated": page.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct RunRecoveryRecipe;

#[async_trait]
impl ToolHandler for RunRecoveryRecipe {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "run_recovery_recipe")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "run_recovery_recipe";
        let recipe = match params["recipe_id"].as_str() {
            Some(raw) => match RecoveryRecipeId::parse(raw) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`recipe_id` is required"),
        };
        let Some(expected_plan_digest) = params["expected_plan_digest"]
            .as_str()
            .map(|d| d.trim().to_string())
        else {
            return ToolResult::err(
                "`expected_plan_digest` is required: it is the reviewed plan you approved, and a \
                 recipe edited since then is a different set of privileged steps",
            );
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.health(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = HealthRequest {
            action: tool.to_string(),
            params,
            recipe,
            expected_plan_digest,
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(tool, &resolved.runtime, provider, call, &request, &desired, &plan).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Backup and scan (Task 5.5)
// ─────────────────────────────────────────────────────────────────────────────

async fn run_job(ctx: &ToolContext, tool: &str, params: serde_json::Value, op: JobOp) -> ToolResult {
    let resolved = match gov::resolve(ctx, tool) {
        Ok(resolved) => resolved,
        Err(result) => return result,
    };
    let provider = match resolved.runtime.backup_scan(tool) {
        Ok(provider) => provider,
        Err(error) => return gov::os_error(&error),
    };
    let call = match gov::mutation_call(ctx, &resolved.runtime, tool) {
        Ok(call) => call,
        Err(result) => return result,
    };
    let request = JobRequest {
        action: tool.to_string(),
        params,
        op,
    };
    let observed = match crate::os_control::contract::DesiredStateControl::observe(
        provider,
        call.observation(),
        &request,
    )
    .await
    {
        Ok(observed) => observed,
        Err(error) => return gov::os_error(&error),
    };
    let desired = request.desired_state(&observed);
    let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
    gov::run_mutation(tool, &resolved.runtime, provider, call, &request, &desired, &plan).await
}

struct GetBackupStatus;

#[async_trait]
impl ToolHandler for GetBackupStatus {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_backup_status")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "get_backup_status";
        let provider_id = match params["provider"].as_str() {
            Some(raw) => match BackupProviderId::parse(raw) {
                Ok(id) => Some(id),
                Err(error) => return gov::os_error(&error),
            },
            None => None,
        };
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.backup_scan(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.status(call.observation(), provider_id).await {
            Ok(status) => ToolResult::ok(serde_json::json!({
                "provider": status.provider.tag(),
                "configured": status.configured,
                "running": status.running,
                // null with configured=false means "never set up"; null with
                // configured=true means the last run time is unknown.
                "last_success_unix": status.last_success_unix,
                "snapshot_count": status.snapshot_count,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct StartBackup;

#[async_trait]
impl ToolHandler for StartBackup {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "start_backup")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "start_backup";
        let provider_id = match params["provider"].as_str() {
            Some(raw) => match BackupProviderId::parse(raw) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`provider` is required"),
        };
        let Some(plan_digest) = params["plan_digest"].as_str().map(|d| d.trim().to_string()) else {
            return ToolResult::err("`plan_digest` is required (the plan you reviewed)");
        };
        run_job(
            &ctx,
            tool,
            params,
            JobOp::StartBackup {
                provider: provider_id,
                plan_digest,
            },
        )
        .await
    }
}

struct PlanBackupRestoreHandoff;

#[async_trait]
impl ToolHandler for PlanBackupRestoreHandoff {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "plan_backup_restore_handoff")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "plan_backup_restore_handoff";
        let provider_id = match params["provider"].as_str() {
            Some(raw) => match BackupProviderId::parse(raw) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`provider` is required"),
        };
        let snapshot = match params["snapshot"].as_str() {
            Some(raw) => match BackupSnapshotId::parse(raw) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`snapshot` is required"),
        };
        let destination = match params["destination"].as_str() {
            Some(_) => match required_path(&params, "destination") {
                Ok(path) => Some(path),
                Err(result) => return result,
            },
            None => None,
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.backup_scan(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        // A read admission: this PLANS a restore and hands off. KRIA never performs
        // the restore, because a wrong snapshot or destination silently destroys
        // the user's current files.
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider
            .plan_restore(
                call.observation(),
                provider_id,
                &snapshot,
                destination.as_ref(),
            )
            .await
        {
            Ok(plan) => ToolResult::ok(serde_json::json!({
                "provider": plan.provider.tag(),
                "snapshot": plan.snapshot.as_str(),
                "destination": plan.destination.as_ref().map(|p| p.to_string_lossy()),
                "handoff": plan.handoff_hint.as_str(),
                "note": "this is a plan only — KRIA does not perform restores, because a wrong snapshot would overwrite your current files",
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct ListScanners;

#[async_trait]
impl ToolHandler for ListScanners {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "list_scanners")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "list_scanners";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.backup_scan(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let limit = params["limit"].as_u64().and_then(|v| usize::try_from(v).ok());
        match provider
            .scanners(call.observation(), params["cursor"].as_str(), limit)
            .await
        {
            Ok(rows) => ToolResult::ok(serde_json::json!({
                "scanners": rows.iter().map(|s| serde_json::json!({
                    "scanner": s.scanner.as_str(),
                    "label": s.label.as_str(),
                })).collect::<Vec<_>>(),
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct ScanDocument;

#[async_trait]
impl ToolHandler for ScanDocument {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "scan_document")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "scan_document";
        let scanner = match params["scanner"].as_str() {
            Some(raw) => match ScannerId::parse(raw) {
                Ok(id) => id,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`scanner` is required"),
        };
        let destination = match required_path(&params, "destination") {
            Ok(path) => path,
            Err(result) => return result,
        };
        let format = match params["format"].as_str() {
            Some(raw) => match ScanFormat::parse(raw) {
                Ok(format) => format,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`format` is required (png, jpeg, pdf)"),
        };
        let dpi = match params["resolution_dpi"].as_u64() {
            Some(raw) => match BoundedDpi::parse(u32::try_from(raw).unwrap_or(0)) {
                Ok(dpi) => dpi,
                Err(error) => return gov::os_error(&error),
            },
            None => return ToolResult::err("`resolution_dpi` is required"),
        };
        let pages = u16::try_from(params["pages"].as_u64().unwrap_or(1)).unwrap_or(1);
        if pages == 0 || pages > 500 {
            return ToolResult::err("`pages` must be between 1 and 500");
        }
        // The domain refuses to overwrite an existing destination.
        run_job(
            &ctx,
            tool,
            params,
            JobOp::ScanDocument {
                scanner,
                destination,
                format,
                dpi,
                pages,
            },
        )
        .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Firmware and sensors (Task 5.4)
// ─────────────────────────────────────────────────────────────────────────────

struct GetFirmwareStatus;

#[async_trait]
impl ToolHandler for GetFirmwareStatus {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_firmware_status")
    }

    async fn execute_with_context(&self, _p: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "get_firmware_status";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.firmware(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.status(call.observation()).await {
            Ok(status) => ToolResult::ok(serde_json::json!({
                "update_source_reachable": status.update_source_reachable,
                "updates_available": status.updates_available(),
                // Devices whose update state could NOT be determined. Never folded
                // into "up to date": that would be a false assurance about a
                // security-relevant component.
                "undetermined": status.undetermined(),
                "devices": status.devices.iter().map(|d| serde_json::json!({
                    "device": d.device,
                    "label": d.label.as_str(),
                    "installed_version": d.installed_version,
                    "available_version": d.available_version,
                    "needs_reboot": d.needs_reboot,
                })).collect::<Vec<_>>(),
                "note": "KRIA reads firmware state only; it never flashes firmware",
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct GetHardwareSensors;

#[async_trait]
impl ToolHandler for GetHardwareSensors {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "get_hardware_sensors")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "get_hardware_sensors";
        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.hardware(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let limit = params["limit"].as_u64().and_then(|v| usize::try_from(v).ok());
        match provider
            .sensors(call.observation(), params["cursor"].as_str(), limit)
            .await
        {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "sensors": page.items.iter().map(|s| serde_json::json!({
                    "sensor": s.sensor,
                    "label": s.label.as_str(),
                    "kind": s.kind.tag(),
                    "unit": s.kind.unit(),
                    // Tenths keep the value exact; a client divides for display.
                    "value_tenths": s.value_tenths,
                    // null when the driver reports no limit — no verdict is invented.
                    "over_threshold": s.over_threshold(),
                })).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor.as_deref(),
                "truncated": page.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Apply system updates (Task 4.4)
// ─────────────────────────────────────────────────────────────────────────────

struct ApplySystemUpdates;

#[async_trait]
impl ToolHandler for ApplySystemUpdates {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "apply_system_updates")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "apply_system_updates";
        let Some(approved_digest) = params["plan_digest"].as_str().map(|d| d.trim().to_string())
        else {
            return ToolResult::err(
                "`plan_digest` is required: it is the update plan you approved, and a plan that \
                 changed since then is a different set of packages",
            );
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.packages(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };

        // Re-derive the plan from the live system, then refuse if it no longer
        // matches what was approved. Between approval and now the mirror may have
        // moved on, and applying a drifted plan would install packages the user
        // never saw.
        let plan = match provider
            .plan(
                call.observation(),
                crate::os_control::packages::PackageOperation::Update,
                &[],
            )
            .await
        {
            Ok(plan) => plan,
            Err(error) => return gov::os_error(&error),
        };
        if approved_digest != plan.digest().as_hex() {
            return ToolResult::err(
                "PLAN_DIGEST_MISMATCH: the available updates changed since you approved this plan; \
                 re-check updates and approve the new plan",
            );
        }

        let request = crate::os_control::packages::PackageRequest {
            action: tool.to_string(),
            params: params.clone(),
            plan,
        };
        let desired = request.desired_state();
        let plan_meta = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            tool,
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan_meta,
        )
        .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Battery charge thresholds (Task 5.4)
// ─────────────────────────────────────────────────────────────────────────────

struct SetBatteryChargeThresholds;

#[async_trait]
impl ToolHandler for SetBatteryChargeThresholds {
    async fn execute(&self, _p: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "set_battery_charge_thresholds")
    }

    async fn execute_with_context(&self, params: serde_json::Value, ctx: ToolContext) -> ToolResult {
        let tool = "set_battery_charge_thresholds";
        let Some(lower) = params["lower"].as_u64().and_then(|v| u8::try_from(v).ok()) else {
            return ToolResult::err("`lower` must be a percentage 0-100");
        };
        let Some(upper) = params["upper"].as_u64().and_then(|v| u8::try_from(v).ok()) else {
            return ToolResult::err("`upper` must be a percentage 0-100");
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.charge_thresholds(tool) {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        // The pair is validated together: writing them one at a time can leave the
        // machine with only one value applied.
        let request = match crate::os_control::power::charge::ChargeThresholdRequest::new(
            tool,
            params,
            lower,
            upper,
        ) {
            Ok(request) => request,
            Err(error) => return gov::os_error(&error),
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(tool, &resolved.runtime, provider, call, &request, &desired, &plan).await
    }
}

/// Register this tool surface.
pub fn register(registry: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "set_battery_charge_thresholds".into(),
                description: "Limit how full the battery charges, to reduce wear".into(),
                category: "hardware".into(),
                // Privileged: written through the broker, not in-process.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("lower", "integer", "Start charging below this percent", true),
                    param("upper", "integer", "Stop charging at this percent (min 20)", true),
                ],
            },
            Arc::new(SetBatteryChargeThresholds),
        ),
        (
            ToolDef {
                name: "apply_system_updates".into(),
                description: "Install the approved set of system updates".into(),
                category: "packages".into(),
                // Installs code system-wide and may require a reboot.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param(
                    "plan_digest",
                    "string",
                    "The update plan you approved; a drifted plan is refused",
                    true,
                )],
            },
            Arc::new(ApplySystemUpdates),
        ),
        (
            ToolDef {
                name: "search_desktop".into(),
                description: "Search indexed files by name, or by content when the scope allows"
                    .into(),
                category: "search".into(),
                // RED when the resolved scope indexes CONTENT; the stricter default
                // is declared here and the gate re-evaluates per call.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("query", "string", "What to search for", true),
                    param("scope", "string", "Search scope id", false),
                    param("cursor", "string", "Page cursor", false),
                    param("limit", "integer", "Maximum results (max 256)", false),
                ],
            },
            Arc::new(SearchDesktop),
        ),
        (
            ToolDef {
                name: "get_search_scope".into(),
                description: "Read which folders are indexed and whether contents are indexed"
                    .into(),
                category: "search".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetSearchScope),
        ),
        (
            ToolDef {
                name: "configure_search_scope".into(),
                description: "Set which folders the desktop search indexes".into(),
                category: "search".into(),
                // Widening the scope widens what every future search can reach.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("roots", "array", "Absolute paths to index (1-256)", true),
                    param("exclusions", "array", "Absolute paths to exclude (max 64)", false),
                ],
            },
            Arc::new(ConfigureSearchScope),
        ),
        (
            ToolDef {
                name: "rebuild_search_index".into(),
                description: "Rebuild the search index — verified as started, not finished".into(),
                category: "search".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param("scope", "string", "Scope to rebuild", false)],
            },
            Arc::new(RebuildSearchIndex),
        ),
        (
            ToolDef {
                name: "diagnose_system".into(),
                description: "Check storage, memory, services, thermal and network health".into(),
                category: "health".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param(
                    "scope",
                    "string",
                    "One subsystem: storage, memory, services, thermal, network",
                    false,
                )],
            },
            Arc::new(DiagnoseSystem),
        ),
        (
            ToolDef {
                name: "get_system_logs".into(),
                description: "Read a bounded, scoped window of system logs".into(),
                category: "health".into(),
                // The journal carries auth failures and other users' activity.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param(
                    "query",
                    "object",
                    "{unit, since_hours (1-24), max_lines (1-500), max_priority (0-7)}",
                    true,
                )],
            },
            Arc::new(GetSystemLogs),
        ),
        (
            ToolDef {
                name: "run_recovery_recipe".into(),
                description: "Run a reviewed in-tree recovery recipe".into(),
                category: "health".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("recipe_id", "string", "A reviewed in-tree recipe id", true),
                    param(
                        "expected_plan_digest",
                        "string",
                        "The plan digest you reviewed; a changed recipe is refused",
                        true,
                    ),
                ],
            },
            Arc::new(RunRecoveryRecipe),
        ),
        (
            ToolDef {
                name: "get_backup_status".into(),
                description: "Read whether backups are configured, running, and when they last ran"
                    .into(),
                category: "backup".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("provider", "string", "deja-dup, timeshift, borg", false)],
            },
            Arc::new(GetBackupStatus),
        ),
        (
            ToolDef {
                name: "start_backup".into(),
                description: "Start a backup — verified as accepted, not as finished".into(),
                category: "backup".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("provider", "string", "deja-dup, timeshift, borg", true),
                    param("plan_digest", "string", "The plan you reviewed", true),
                ],
            },
            Arc::new(StartBackup),
        ),
        (
            ToolDef {
                name: "plan_backup_restore_handoff".into(),
                description: "Plan a restore and hand off — KRIA never performs restores".into(),
                category: "backup".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("provider", "string", "deja-dup, timeshift, borg", true),
                    param("snapshot", "string", "The snapshot to restore from", true),
                    param("destination", "string", "Where a restore would write", false),
                ],
            },
            Arc::new(PlanBackupRestoreHandoff),
        ),
        (
            ToolDef {
                name: "list_scanners".into(),
                description: "List connected scanners".into(),
                category: "scan".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("cursor", "string", "Page cursor", false),
                    param("limit", "integer", "Maximum rows (max 256)", false),
                ],
            },
            Arc::new(ListScanners),
        ),
        (
            ToolDef {
                name: "scan_document".into(),
                description: "Scan to a file — refuses to overwrite an existing one".into(),
                category: "scan".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("scanner", "string", "Scanner device name", true),
                    param("destination", "string", "Absolute output path", true),
                    param("format", "string", "png, jpeg, or pdf", true),
                    param("resolution_dpi", "integer", "75-1200", true),
                    param("pages", "integer", "1-500", false),
                ],
            },
            Arc::new(ScanDocument),
        ),
        (
            ToolDef {
                name: "get_firmware_status".into(),
                description: "Read firmware versions and available updates (read-only)".into(),
                category: "hardware".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetFirmwareStatus),
        ),
        (
            ToolDef {
                name: "get_hardware_sensors".into(),
                description: "Read temperatures, fan speeds, voltages and power draw".into(),
                category: "hardware".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("cursor", "string", "Page cursor", false),
                    param("limit", "integer", "Maximum rows (max 256)", false),
                ],
            },
            Arc::new(GetHardwareSensors),
        ),
    ];

    for (def, handler) in tools {
        registry.register(def, handler);
    }
}
