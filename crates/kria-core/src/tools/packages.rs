//! Package tools: `search_package`, `get_package_info`,
//! `list_installed_packages`, `plan_package_changes`, `install_package`,
//! `uninstall_package`, `check_system_updates`, `get_reboot_required`.
//!
//! linux-os-control-production **Task 3.4** — "Complete package planning,
//! install/remove and update assessment" (OSC-014).
//!
//! Every handler here is a **thin facade**: it reaches host effects **only**
//! through the injected [`OsControlRuntime`] +
//! `os_control::packages::PackageControl` provider — never a direct
//! subprocess. This replaces the previous ~1700-line direct-execution
//! implementation that shelled out to `apt`/`dnf`/`pacman`/`zypper`/`brew`/
//! `winget`/`choco`/`snap`/`flatpak` and escalated privilege itself via an
//! ad-hoc `PrivEsc`/`pkexec`/`sudo` invocation. That machinery is deleted
//! outright (per dev-context.md: delete dead code directly, no shims) —
//! privileged package mutation now dispatches exclusively through
//! `BrokerOperation::ApplyPackagePlan` bound to the approved plan digest,
//! from inside `os_control::packages::PackageControl::apply` (never from
//! this file).
//!
//! Until a live PackageKit/distro-adapter transport is composed into the
//! runtime (desktop startup root), every handler fails closed with the
//! frozen `Unavailable` envelope and never falls back to an ungoverned
//! subprocess or `pkexec`/`sudo` invocation.
//!
//! The `check_package_installed`/`check_package_updates` legacy tool names
//! are retired per the frozen legacy-difference report (folded into
//! `get_package_info`/`list_installed_packages`, and renamed to
//! `check_system_updates`, respectively) — they are not re-registered here.

use crate::infra::ToolResult;
use crate::os_control::contract::SafeText;
use crate::os_control::packages::{PackageOperation, PackageProviderId, PackageRef};
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

/// Return the governed OS-control `Unavailable` envelope for a package tool.
///
/// Every migrated package handler reaches host effects **only** through the
/// injected [`OsControlRuntime`] + `os_control::packages::PackageControl`
/// provider — never a direct `apt`/`dnf`/`pacman`/`zypper`/`snap`/
/// `flatpak`/`pkexec`/`sudo` subprocess (Task 3.4 completion proof). Until a
/// live provider is composed into the runtime, the handler fails closed
/// with this frozen envelope.
fn os_packages_unavailable(runtime: Option<&Arc<OsControlRuntime>>, tool: &str) -> ToolResult {
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

/// Parse a `{provider, name}` `PackageRef` param object, defaulting the
/// provider to `apt` when omitted (the common Ubuntu case) so canonical
/// calls that pass only a bare package name remain ergonomic.
fn parse_package_ref(value: &serde_json::Value) -> Result<PackageRef, ToolResult> {
    let name = value["name"]
        .as_str()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| ToolResult::err("package.name is required"))?;
    let provider = match value["provider"].as_str() {
        Some(p) => PackageProviderId::from_str_lossy(p)
            .ok_or_else(|| ToolResult::err(format!("unknown package provider '{p}'")))?,
        None => PackageProviderId::Apt,
    };
    Ok(PackageRef::new(provider, name))
}

fn parse_optional_provider(
    params: &serde_json::Value,
) -> Result<Option<PackageProviderId>, ToolResult> {
    match params["provider"].as_str() {
        None => Ok(None),
        Some(p) => PackageProviderId::from_str_lossy(p)
            .map(Some)
            .ok_or_else(|| ToolResult::err(format!("unknown package provider '{p}'"))),
    }
}

// ─── search_package (GREEN) ────────────────────────────────────────────────

struct SearchPackage;

#[async_trait]
impl ToolHandler for SearchPackage {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_packages_unavailable(None, "search_package")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let query = params["query"]
            .as_str()
            .or_else(|| params["name"].as_str())
            .map(str::trim)
            .filter(|q| !q.is_empty());
        if query.is_none() {
            return ToolResult::err("query parameter is required (or provide name as alias)");
        }
        if let Err(err) = parse_optional_provider(&params) {
            return err;
        }
        // The governed PackageControl provider owns the actual
        // PackageKit/distro-adapter search across available providers.
        let resolved = match gov::resolve(&ctx, "search_package") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.packages("search_package") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "search_package") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let query = params["query"].as_str().unwrap_or_default();
        let limit = params["limit"].as_u64().unwrap_or(25).min(200) as usize;
        match provider.search(call.observation(), query, None, 0, limit).await {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "packages": page.items.iter().map(|e| serde_json::json!({
                        "name": e.package.name(),
                        "provider": e.provider.as_str(),
                        "installed_version": e.installed_version,
                        "candidate_version": e.candidate_version,
                    })).collect::<Vec<_>>(),
                "truncated": page.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─── get_package_info (GREEN) ──────────────────────────────────────────────

struct GetPackageInfo;

#[async_trait]
impl ToolHandler for GetPackageInfo {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_packages_unavailable(None, "get_package_info")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if let Err(err) = parse_package_ref(&params["package"]) {
            return err;
        }
        // The governed PackageControl provider owns the actual normalized
        // package observation read through the runtime.
        let resolved = match gov::resolve(&ctx, "get_package_info") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.packages("get_package_info") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "get_package_info") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let name = params["name"]
            .as_str()
            .or_else(|| params["package"].as_str())
            .unwrap_or_default();
        // PackageKit fronts whichever native manager the host uses, so it is the
        // provider-neutral reference for a lookup.
        let package = crate::os_control::packages::PackageRef::new(
            crate::os_control::packages::PackageProviderId::PackageKit,
            name,
        );
        match provider.get_info(call.observation(), &package).await {
            Ok(o) => ToolResult::ok(serde_json::json!({
                "name": o.package.name(),
                "provider": o.provider.as_str(),
                "installed_version": o.installed_version,
                "candidate_version": o.candidate_version,
                "origin": o.origin,
                "size_bytes": o.size_bytes,
                "dependency_count": o.dependency_count,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─── list_installed_packages (GREEN) ───────────────────────────────────────

struct ListInstalledPackages;

#[async_trait]
impl ToolHandler for ListInstalledPackages {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_packages_unavailable(None, "list_installed_packages")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if let Err(err) = parse_optional_provider(&params) {
            return err;
        }
        // The governed PackageControl provider owns the actual installed-
        // package listing across available providers.
        let resolved = match gov::resolve(&ctx, "list_installed_packages") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.packages("list_installed_packages") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "list_installed_packages") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let limit = params["limit"].as_u64().unwrap_or(100).min(500) as usize;
        let cursor = params["cursor"].as_u64().unwrap_or(0) as usize;
        match provider.list_installed(call.observation(), None, cursor, limit).await {
            Ok(page) => ToolResult::ok(serde_json::json!({
                "packages": page.items.iter().map(|e| serde_json::json!({
                        "name": e.package.name(),
                        "provider": e.provider.as_str(),
                        "installed_version": e.installed_version,
                        "candidate_version": e.candidate_version,
                    })).collect::<Vec<_>>(),
                "truncated": page.truncated,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─── plan_package_changes (GREEN) ──────────────────────────────────────────

struct PlanPackageChanges;

#[async_trait]
impl ToolHandler for PlanPackageChanges {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_packages_unavailable(None, "plan_package_changes")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let operation = match params["operation"].as_str() {
            Some("install") => PackageOperation::Install,
            Some("remove") => PackageOperation::Remove,
            Some("update") => PackageOperation::Update,
            Some(other) => {
                return ToolResult::err(format!(
                    "unknown operation '{other}': expected install, remove, or update"
                ))
            }
            None => return ToolResult::err("operation parameter is required"),
        };
        let packages = match params["packages"].as_array() {
            Some(items) if !items.is_empty() => {
                let mut refs = Vec::with_capacity(items.len());
                for item in items {
                    match parse_package_ref(item) {
                        Ok(r) => refs.push(r),
                        Err(err) => return err,
                    }
                }
                refs
            }
            _ => return ToolResult::err("packages parameter must be a non-empty array"),
        };
        let _ = (operation, packages);
        // The governed PackageControl provider owns building the exact
        // preflight plan (install/upgrade/removal split, download/disk-
        // delta/security/reboot metadata) through the runtime. This is the
        // only place install-vs-update-vs-remove-vs-no-change is resolved.
        let resolved = match gov::resolve(&ctx, "plan_package_changes") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.packages("plan_package_changes") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "plan_package_changes") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let operation = match params["operation"].as_str().unwrap_or("install") {
            "remove" | "uninstall" => crate::os_control::packages::PackageOperation::Remove,
            "update" | "upgrade" => crate::os_control::packages::PackageOperation::Update,
            _ => crate::os_control::packages::PackageOperation::Install,
        };
        let refs = package_refs(&params);
        match provider.plan(call.observation(), operation, &refs).await {
            Ok(plan) => ToolResult::ok(serde_json::json!({
                "plan_digest": plan.digest().as_hex(),
                "operation": plan.operation.as_str(),
                "provider": plan.provider.as_str(),
                "installs": plan.installs.len(),
                "upgrades": plan.upgrades.len(),
                "removals": plan.removals.len(),
                "download_bytes": plan.download_bytes,
                "disk_delta_bytes": plan.disk_delta_bytes,
                "security_relevant": plan.security_relevant,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─── install_package (RED) ─────────────────────────────────────────────────

struct InstallPackage;

#[async_trait]
impl ToolHandler for InstallPackage {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_packages_unavailable(None, "install_package")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let plan_digest = params["plan_digest"].as_str().unwrap_or("").trim();
        if plan_digest.is_empty() {
            return ToolResult::err(
                "plan_digest parameter is required (call plan_package_changes first)",
            );
        }
        // The governed PackageControl provider applies the approved,
        // digest-bound plan exclusively through
        // `BrokerOperation::ApplyPackagePlan` — never a direct pkexec/sudo
        // subprocess from this handler.
        let resolved = match gov::resolve(&ctx, "install_package") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.packages("install_package") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "install_package") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let refs = package_refs(&params);
        // Build the plan under the observation authority first, then refuse if it
        // no longer matches the digest the caller approved — a plan that drifted
        // since approval must not be applied silently.
        let plan = match provider
            .plan(
                call.observation(),
                crate::os_control::packages::PackageOperation::Install,
                &refs,
            )
            .await
        {
            Ok(plan) => plan,
            Err(error) => return gov::os_error(&error),
        };
        if let Some(approved) = params["plan_digest"].as_str() {
            if approved != plan.digest().as_hex() {
                return ToolResult::err(
                    "PLAN_DIGEST_MISMATCH: the resolved package plan changed since approval",
                );
            }
        }
        let request = crate::os_control::packages::PackageRequest {
            action: "install_package".to_string(),
            params: params.clone(),
            plan,
        };
        let desired = request.desired_state();
        let plan_meta = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "install_package",
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

// ─── uninstall_package (RED) ───────────────────────────────────────────────

struct UninstallPackage;

#[async_trait]
impl ToolHandler for UninstallPackage {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_packages_unavailable(None, "uninstall_package")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let plan_digest = params["plan_digest"].as_str().unwrap_or("").trim();
        if plan_digest.is_empty() {
            return ToolResult::err(
                "plan_digest parameter is required (call plan_package_changes first)",
            );
        }
        // Same closed plan-apply operation as `install_package`, dispatched
        // exclusively through `BrokerOperation::ApplyPackagePlan`.
        let resolved = match gov::resolve(&ctx, "uninstall_package") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.packages("uninstall_package") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "uninstall_package") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let refs = package_refs(&params);
        // Build the plan under the observation authority first, then refuse if it
        // no longer matches the digest the caller approved — a plan that drifted
        // since approval must not be applied silently.
        let plan = match provider
            .plan(
                call.observation(),
                crate::os_control::packages::PackageOperation::Remove,
                &refs,
            )
            .await
        {
            Ok(plan) => plan,
            Err(error) => return gov::os_error(&error),
        };
        if let Some(approved) = params["plan_digest"].as_str() {
            if approved != plan.digest().as_hex() {
                return ToolResult::err(
                    "PLAN_DIGEST_MISMATCH: the resolved package plan changed since approval",
                );
            }
        }
        let request = crate::os_control::packages::PackageRequest {
            action: "uninstall_package".to_string(),
            params: params.clone(),
            plan,
        };
        let desired = request.desired_state();
        let plan_meta = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "uninstall_package",
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

// ─── check_system_updates (GREEN) ──────────────────────────────────────────

struct CheckSystemUpdates;

#[async_trait]
impl ToolHandler for CheckSystemUpdates {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_packages_unavailable(None, "check_system_updates")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if let Err(err) = parse_optional_provider(&params) {
            return err;
        }
        // The governed PackageControl provider owns the actual routine
        // update assessment (security relevance, reboot likelihood) — never
        // a fabricated guess when the provider doesn't supply it.
        let resolved = match gov::resolve(&ctx, "check_system_updates") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.packages("check_system_updates") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "check_system_updates") {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.assess_updates(call.observation(), None).await {
            Ok(a) => ToolResult::ok(serde_json::json!({
                "provider": a.provider.as_str(),
                "update_count": a.update_count,
                "security_update_count": a.security_update_count,
                "download_bytes": a.download_bytes,
                "reboot_likely": a.reboot_likely,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─── get_reboot_required (GREEN) ───────────────────────────────────────────

struct GetRebootRequired;

#[async_trait]
impl ToolHandler for GetRebootRequired {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_packages_unavailable(None, "get_reboot_required")
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // The governed PackageControl provider owns the actual current
        // reboot-required query through the runtime.
        let resolved = match gov::resolve(&ctx, "get_reboot_required") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.packages("get_reboot_required") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, "get_reboot_required") {
            Ok(call) => call,
            Err(result) => return result,
        };
        match provider.reboot_required(call.observation()).await {
            Ok(r) => ToolResult::ok(serde_json::json!({
                "required": r.required,
                "reason_count": r.reason_count,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

// ─── Register all tools ─────────────────────────────────────────────────────

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        // GREEN: query/planning tools (auto-execute, no approval needed)
        (
            ToolDef {
                name: "search_package".into(),
                description: "Search for OPERATING-SYSTEM software packages/applications across the system package providers (PackageKit/apt/dnf/pacman/zypper/snap/flatpak) — e.g. htop, docker, vlc. This is ONLY for OS software, NOT for KRIA skills/tools/capabilities (for those use `search_marketplace`). Returns matching package names, versions, origins and providers. Always call this before installing an OS package.".into(),
                category: "packages".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("query", "string", "Package name or keyword to search for", true),
                    param("name", "string", "Alias for query (for compatibility with older calls)", false),
                    param("provider", "string", "Specific provider to search: packagekit, apt, dnf, pacman, zypper, snap, flatpak. Omit to search all available providers.", false),
                    param("cursor", "string", "Pagination cursor from a previous call", false),
                    param("limit", "integer", "Maximum results per page", false),
                ],
            },
            Arc::new(SearchPackage),
        ),
        (
            ToolDef {
                name: "get_package_info".into(),
                description: "Get the normalized observation for one package: installed version, candidate version, origin, size, and dependency/reboot summary. Use this to verify a package before installing.".into(),
                category: "packages".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("package", "object", "Package identity {provider, name} (from search_package)", true),
                ],
            },
            Arc::new(GetPackageInfo),
        ),
        (
            ToolDef {
                name: "list_installed_packages".into(),
                description: "List installed packages/apps across available providers (PackageKit/apt/dnf/pacman/zypper/snap/flatpak). Read-only. Use for 'list installed apps', 'show all packages', etc.".into(),
                category: "packages".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("provider", "string", "Specific provider to list: packagekit, apt, dnf, pacman, zypper, snap, flatpak. Omit to list across all available providers.", false),
                    param("cursor", "string", "Pagination cursor from a previous call", false),
                    param("limit", "integer", "Maximum results per page", false),
                ],
            },
            Arc::new(ListInstalledPackages),
        ),
        (
            ToolDef {
                name: "plan_package_changes".into(),
                description: "Build the exact preflight plan for installing, removing, or updating one or more packages: the resolved install/upgrade/removal split plus download size, disk delta, security relevance, and reboot requirement. Call this before install_package/uninstall_package and show the plan to the user.".into(),
                category: "packages".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("operation", "string", "One of: install, remove, update", true),
                    param("packages", "array", "Package identities {provider, name} to plan changes for", true),
                ],
            },
            Arc::new(PlanPackageChanges),
        ),
        (
            ToolDef {
                name: "check_system_updates".into(),
                description: "Assess routine available updates: count, security relevance (when known), download size, and reboot likelihood. Never fabricates security/reboot metadata the provider does not supply.".into(),
                category: "packages".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("provider", "string", "Specific provider to assess: packagekit, apt, dnf, pacman, zypper, snap, flatpak. Omit to assess across all available providers.", false),
                ],
            },
            Arc::new(CheckSystemUpdates),
        ),
        (
            ToolDef {
                name: "get_reboot_required".into(),
                description: "Check whether a reboot is currently required to complete already-applied package changes.".into(),
                category: "packages".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetRebootRequired),
        ),
        // RED: action tools (require HITL approval)
        (
            ToolDef {
                name: "install_package".into(),
                description: "Apply a previously built, approved package plan to install packages. Requires calling plan_package_changes first and passing its plan_digest. Requires user approval.".into(),
                category: "packages".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    param("plan_digest", "string", "The exact plan digest returned by plan_package_changes", true),
                ],
            },
            Arc::new(InstallPackage),
        ),
        (
            ToolDef {
                name: "uninstall_package".into(),
                description: "Apply a previously built, approved package plan to remove packages. Requires calling plan_package_changes first and passing its plan_digest. Requires user approval.".into(),
                category: "packages".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    param("plan_digest", "string", "The exact plan digest returned by plan_package_changes", true),
                ],
            },
            Arc::new(UninstallPackage),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}

/// Parse the canonical package list into provider-neutral references.
///
/// PackageKit fronts whichever native manager the host uses, so a caller never
/// has to name apt/dnf/pacman explicitly.
fn package_refs(params: &serde_json::Value) -> Vec<crate::os_control::packages::PackageRef> {
    let provider = crate::os_control::packages::PackageProviderId::PackageKit;
    let single = params["name"].as_str().or_else(|| params["package"].as_str());
    if let Some(name) = single {
        return vec![crate::os_control::packages::PackageRef::new(provider, name)];
    }
    params["packages"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(|name| crate::os_control::packages::PackageRef::new(provider, name))
                .collect()
        })
        .unwrap_or_default()
}
