//! Execution Environment Resolution — Phase 1 Migration Layer.
//!
//! This module provides the transitional bridge between the legacy `ExecutionTarget`
//! enum (which conflates environment + category) and the new canonical
//! `ExecutionEnvironment` + `ToolCategory` model from `workflow_types.rs`.
//!
//! # The Problem Being Solved
//!
//! The legacy `ExecutionTarget` has 7 variants:
//!   Host, Vm, Docker, Colab, Browser, Mcp, CloudProvider
//!
//! Of these, only 4 are actual execution environments (where processes run):
//!   Host, Vm, Docker, Colab
//!
//! The other 3 are tool categories (what kind of operation):
//!   Browser → tool that opens/controls a browser (runs on Host)
//!   Mcp → tool that calls an MCP server (runs on Host)
//!   CloudProvider → tool that calls a cloud API (runs on Host)
//!
//! This conflation caused `EXECUTION_BLOCKED` errors when:
//! - `managed_browser_navigate` resolved to target=Browser
//! - But the policy said allowed_targets=[Host] (before the fix)
//! - Even after the fix (allowed=[Browser, Host]), the semantics are wrong:
//!   the browser process runs on Host, not in some "Browser environment"
//!
//! # Solution
//!
//! 1. Map every `ExecutionTarget` to `(ExecutionEnvironment, Option<ToolCategory>)`
//! 2. Provide a new validation function that checks environment only
//! 3. Keep the old `ExecutionTarget` alive for backward compatibility
//! 4. New code uses `ExecutionEnvironment` directly
//!
//! # Migration Path
//!
//! Phase 1 (this file): Adapter functions, new validation, tests
//! Phase 2 (later): Callers migrate to new types
//! Phase 3 (later): Remove legacy `ExecutionTarget::Browser/Mcp/CloudProvider`

use crate::agent::turn_memory::ExecutionTarget;
use crate::agent::workflow_types::{ExecutionEnvironment, ToolCategory};

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Legacy → New Type Mapping
// ═══════════════════════════════════════════════════════════════════════════════

/// Map a legacy `ExecutionTarget` to the correct `ExecutionEnvironment`.
///
/// Browser, Mcp, and CloudProvider all physically execute on the Host.
/// This is the key insight that eliminates the category error.
pub fn to_environment(target: ExecutionTarget) -> ExecutionEnvironment {
    match target {
        ExecutionTarget::Host => ExecutionEnvironment::Host,
        ExecutionTarget::Vm => ExecutionEnvironment::Vm,
        ExecutionTarget::Docker => ExecutionEnvironment::Docker,
        ExecutionTarget::Colab => ExecutionEnvironment::Colab,
        // These are NOT environments — they run on Host
        ExecutionTarget::Browser => ExecutionEnvironment::Host,
        ExecutionTarget::Mcp => ExecutionEnvironment::Host,
        ExecutionTarget::CloudProvider => ExecutionEnvironment::Host,
    }
}

/// Extract the tool category implied by a legacy `ExecutionTarget`.
/// Returns `None` for pure environment targets (Host, Vm, Docker, Colab).
pub fn to_category(target: ExecutionTarget) -> Option<ToolCategory> {
    match target {
        ExecutionTarget::Browser => Some(ToolCategory::Browser),
        ExecutionTarget::Mcp => Some(ToolCategory::Mcp),
        ExecutionTarget::CloudProvider => Some(ToolCategory::CloudProvider),
        _ => None,
    }
}

/// Decompose a legacy target into its canonical components.
pub fn decompose(target: ExecutionTarget) -> (ExecutionEnvironment, Option<ToolCategory>) {
    (to_environment(target), to_category(target))
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Tool Category Classification
// ═══════════════════════════════════════════════════════════════════════════════

/// Classify a tool into its category based on name.
/// This is deterministic and does not depend on user text.
pub fn classify_tool_category(tool_name: &str) -> ToolCategory {
    let lower = tool_name.to_ascii_lowercase();

    // Browser tools — these control a browser but run on Host
    if matches!(
        lower.as_str(),
        "open_url"
            | "browser_search"
            | "managed_browser_navigate"
            | "open_application"
            | "open_application_with_file"
            | "close_application"
            | "focus_window"
            | "maximize_window"
            | "minimize_window"
            | "tile_windows"
            | "get_active_window"
            | "list_windows"
            | "screenshot"
            | "screenshot_analyze"
            | "type_text"
            | "click_mouse"
            | "click_element"
            | "press_shortcut"
            | "drag_mouse"
    ) {
        return ToolCategory::Desktop;
    }

    // Pure browser navigation (subset of Desktop but semantically distinct)
    if matches!(
        lower.as_str(),
        "browser_search" | "managed_browser_navigate" | "open_url"
    ) {
        return ToolCategory::Browser;
    }

    // MCP tools
    if lower.starts_with("mcp_") {
        return ToolCategory::Mcp;
    }

    // Cloud provider tools
    if lower.starts_with("gw_") {
        return ToolCategory::CloudProvider;
    }

    // Shell execution
    if matches!(
        lower.as_str(),
        "execute_bash" | "execute_python" | "execute_powershell"
    ) {
        return ToolCategory::Shell;
    }

    // Filesystem
    if matches!(
        lower.as_str(),
        "read_file"
            | "write_file"
            | "delete_file"
            | "move_file"
            | "copy_file"
            | "create_directory"
            | "delete_directory"
            | "list_directory"
            | "search_files"
            | "get_file_info"
            | "rename_file"
            | "create_file"
            | "overwrite_file"
    ) {
        return ToolCategory::Filesystem;
    }

    // Image
    if matches!(
        lower.as_str(),
        "generate_image" | "analyze_image" | "ocr_image" | "image_analyze"
    ) {
        return ToolCategory::Image;
    }

    // Memory
    if matches!(
        lower.as_str(),
        "remember_fact"
            | "recall_fact"
            | "search_knowledge"
            | "save_snippet"
            | "get_snippet"
            | "ingest_document"
            | "rag_query"
    ) {
        return ToolCategory::Memory;
    }

    // Network
    if matches!(
        lower.as_str(),
        "web_search"
            | "searxng_search"
            | "search_news"
            | "fetch_webpage"
            | "fetch_article"
            | "ping_host"
            | "dns_lookup"
            | "download_file"
    ) {
        return ToolCategory::Network;
    }

    // Default: Shell (most tools ultimately run commands)
    ToolCategory::Shell
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Environment-Aware Validation (New Semantics)
// ═══════════════════════════════════════════════════════════════════════════════

/// Environments where a tool is allowed to execute.
/// This replaces the legacy `ToolTargetPolicy.allowed_targets` for new code.
#[derive(Debug, Clone)]
pub struct EnvironmentPolicy {
    /// Which environments this tool can run in
    pub allowed_environments: &'static [ExecutionEnvironment],
    /// Whether this tool is destructive
    pub is_destructive: bool,
    /// Whether ambiguity should block execution
    pub block_on_ambiguity: bool,
    /// Minimum confidence for execution
    pub min_confidence: f32,
}

/// Get the environment policy for a tool.
///
/// Key difference from legacy `policy_for_tool`: Browser/MCP/CloudProvider
/// tools are simply Host-allowed. No more `ExecutionTarget::Browser` in the
/// allowed list — because Browser is not an environment.
pub fn environment_policy_for_tool(tool_name: &str) -> EnvironmentPolicy {
    let lower = tool_name.to_ascii_lowercase();

    // Fleet/VM tools — VM only
    if lower == "execute_fleet_command" {
        return EnvironmentPolicy {
            allowed_environments: &[ExecutionEnvironment::Vm],
            is_destructive: true,
            block_on_ambiguity: true,
            min_confidence: 0.8,
        };
    }
    if lower == "get_fleet_overview" {
        return EnvironmentPolicy {
            allowed_environments: &[ExecutionEnvironment::Vm],
            is_destructive: false,
            block_on_ambiguity: true,
            min_confidence: 0.6,
        };
    }

    // Shell execution — Host, Vm, Docker
    if matches!(
        lower.as_str(),
        "execute_bash" | "execute_python" | "execute_powershell"
    ) {
        return EnvironmentPolicy {
            allowed_environments: &[
                ExecutionEnvironment::Host,
                ExecutionEnvironment::Vm,
                ExecutionEnvironment::Docker,
            ],
            is_destructive: true,
            block_on_ambiguity: true,
            min_confidence: 0.7,
        };
    }

    // Destructive file ops — Host only
    if matches!(
        lower.as_str(),
        "delete_file" | "delete_directory" | "move_file"
    ) {
        return EnvironmentPolicy {
            allowed_environments: &[ExecutionEnvironment::Host],
            is_destructive: true,
            block_on_ambiguity: true,
            min_confidence: 0.7,
        };
    }

    // Package management — Host or Vm
    if matches!(
        lower.as_str(),
        "install_package"
            | "uninstall_package"
            | "install_application"
            | "uninstall_application"
            | "update_all_packages"
    ) {
        return EnvironmentPolicy {
            allowed_environments: &[ExecutionEnvironment::Host, ExecutionEnvironment::Vm],
            is_destructive: true,
            block_on_ambiguity: true,
            min_confidence: 0.7,
        };
    }

    // System control — Host only, destructive
    if matches!(
        lower.as_str(),
        "shutdown_system" | "reboot_system" | "hibernate" | "sleep"
    ) {
        return EnvironmentPolicy {
            allowed_environments: &[ExecutionEnvironment::Host],
            is_destructive: true,
            block_on_ambiguity: true,
            min_confidence: 0.7,
        };
    }

    // Everything else (browser, desktop, MCP, cloud, filesystem, memory, etc.)
    // runs on Host. These tools don't need environment validation because
    // they always run locally — the "target" concept doesn't apply to them.
    EnvironmentPolicy {
        allowed_environments: &[ExecutionEnvironment::Host],
        is_destructive: false,
        block_on_ambiguity: false,
        min_confidence: 0.1,
    }
}

/// Validate execution using the new environment-aware semantics.
///
/// This is the replacement for `validate_binding` that eliminates the
/// Browser/Mcp/CloudProvider category error. It only checks whether the
/// resolved environment is allowed — tool category is irrelevant to
/// environment validation.
///
/// Returns `true` if execution should proceed, `false` if blocked.
pub fn validate_environment(
    tool_name: &str,
    resolved_target: ExecutionTarget,
) -> EnvironmentValidation {
    let env = to_environment(resolved_target);
    let policy = environment_policy_for_tool(tool_name);

    if policy.allowed_environments.contains(&env) {
        EnvironmentValidation::Allowed { environment: env }
    } else {
        let allowed_names: Vec<&str> = policy
            .allowed_environments
            .iter()
            .map(|e| e.as_str())
            .collect();
        EnvironmentValidation::Blocked {
            resolved_environment: env,
            allowed: allowed_names.iter().map(|s| s.to_string()).collect(),
            reason: format!(
                "'{}' cannot execute in environment '{}'. Allowed: [{}]",
                tool_name,
                env.as_str(),
                allowed_names.join(", ")
            ),
        }
    }
}

/// Result of environment validation.
#[derive(Debug, Clone)]
pub enum EnvironmentValidation {
    Allowed { environment: ExecutionEnvironment },
    Blocked {
        resolved_environment: ExecutionEnvironment,
        allowed: Vec<String>,
        reason: String,
    },
}

impl EnvironmentValidation {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Infer Environment from User Text (New Semantics)
// ═══════════════════════════════════════════════════════════════════════════════

/// Infer the execution environment from user text.
/// Unlike `ExecutionTarget::infer`, this ONLY returns real environments.
/// Browser/MCP/Cloud mentions do NOT change the environment — they're categories.
pub fn infer_environment(user_text: &str) -> ExecutionEnvironment {
    let lower = user_text.to_ascii_lowercase();

    // Explicit VM/SSH signals
    if lower.contains(" on my vm")
        || lower.contains(" on vm")
        || lower.contains(" in my vm")
        || lower.contains(" via ssh")
        || lower.starts_with("ssh ")
        || lower.contains("remote machine")
        || lower.contains("remote server")
    {
        return ExecutionEnvironment::Vm;
    }

    // Docker signals
    if lower.contains(" in docker")
        || lower.contains(" in the container")
        || lower.contains(" in container")
        || lower.contains("docker container")
    {
        return ExecutionEnvironment::Docker;
    }

    // Colab signals
    if lower.contains(" in colab")
        || lower.contains(" on colab")
        || lower.contains("colab notebook")
    {
        return ExecutionEnvironment::Colab;
    }

    // Default: Host
    // NOTE: "browser", "chrome", "firefox" do NOT change the environment.
    // The browser process runs on Host. This is the key fix.
    ExecutionEnvironment::Host
}

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Environment mapping ──────────────────────────────────────────────────

    #[test]
    fn browser_target_maps_to_host_environment() {
        assert_eq!(to_environment(ExecutionTarget::Browser), ExecutionEnvironment::Host);
    }

    #[test]
    fn mcp_target_maps_to_host_environment() {
        assert_eq!(to_environment(ExecutionTarget::Mcp), ExecutionEnvironment::Host);
    }

    #[test]
    fn cloud_target_maps_to_host_environment() {
        assert_eq!(to_environment(ExecutionTarget::CloudProvider), ExecutionEnvironment::Host);
    }

    #[test]
    fn vm_target_maps_to_vm_environment() {
        assert_eq!(to_environment(ExecutionTarget::Vm), ExecutionEnvironment::Vm);
    }

    // ── Category classification ──────────────────────────────────────────────

    #[test]
    fn browser_tools_classified_as_desktop() {
        assert_eq!(classify_tool_category("managed_browser_navigate"), ToolCategory::Desktop);
        assert_eq!(classify_tool_category("open_application"), ToolCategory::Desktop);
        assert_eq!(classify_tool_category("type_text"), ToolCategory::Desktop);
    }

    #[test]
    fn mcp_tools_classified_correctly() {
        assert_eq!(classify_tool_category("mcp_filesystem_read"), ToolCategory::Mcp);
        assert_eq!(classify_tool_category("mcp_colab_execute"), ToolCategory::Mcp);
    }

    #[test]
    fn shell_tools_classified_correctly() {
        assert_eq!(classify_tool_category("execute_bash"), ToolCategory::Shell);
    }

    // ── Environment validation (the key fix) ─────────────────────────────────

    #[test]
    fn browser_navigate_on_browser_target_is_allowed() {
        // This is the exact bug that caused EXECUTION_BLOCKED.
        // Browser target → Host environment → Host is always allowed for desktop tools.
        let result = validate_environment("managed_browser_navigate", ExecutionTarget::Browser);
        assert!(result.is_allowed(), "browser tools must not be blocked when target is Browser");
    }

    #[test]
    fn browser_navigate_on_host_target_is_allowed() {
        let result = validate_environment("managed_browser_navigate", ExecutionTarget::Host);
        assert!(result.is_allowed());
    }

    #[test]
    fn fleet_command_on_browser_target_is_blocked() {
        // Browser → Host environment, but fleet_command only allows Vm
        let result = validate_environment("execute_fleet_command", ExecutionTarget::Browser);
        assert!(!result.is_allowed());
    }

    #[test]
    fn fleet_command_on_vm_target_is_allowed() {
        let result = validate_environment("execute_fleet_command", ExecutionTarget::Vm);
        assert!(result.is_allowed());
    }

    #[test]
    fn execute_bash_on_host_is_allowed() {
        let result = validate_environment("execute_bash", ExecutionTarget::Host);
        assert!(result.is_allowed());
    }

    #[test]
    fn execute_bash_on_vm_is_allowed() {
        let result = validate_environment("execute_bash", ExecutionTarget::Vm);
        assert!(result.is_allowed());
    }

    #[test]
    fn delete_file_on_vm_is_blocked() {
        let result = validate_environment("delete_file", ExecutionTarget::Vm);
        assert!(!result.is_allowed());
    }

    // ── Environment inference ────────────────────────────────────────────────

    #[test]
    fn browser_mention_does_not_change_environment() {
        // "Open the browser" should NOT infer a non-Host environment
        assert_eq!(
            infer_environment("Open the browser and go to youtube"),
            ExecutionEnvironment::Host
        );
    }

    #[test]
    fn vm_mention_infers_vm_environment() {
        assert_eq!(
            infer_environment("run this on my VM"),
            ExecutionEnvironment::Vm
        );
    }

    #[test]
    fn docker_mention_infers_docker_environment() {
        assert_eq!(
            infer_environment("run in docker container"),
            ExecutionEnvironment::Docker
        );
    }

    #[test]
    fn plain_text_infers_host() {
        assert_eq!(
            infer_environment("check my cpu usage"),
            ExecutionEnvironment::Host
        );
    }

    // ── Decomposition ────────────────────────────────────────────────────────

    #[test]
    fn decompose_browser_gives_host_plus_browser_category() {
        let (env, cat) = decompose(ExecutionTarget::Browser);
        assert_eq!(env, ExecutionEnvironment::Host);
        assert_eq!(cat, Some(ToolCategory::Browser));
    }

    #[test]
    fn decompose_host_gives_host_plus_no_category() {
        let (env, cat) = decompose(ExecutionTarget::Host);
        assert_eq!(env, ExecutionEnvironment::Host);
        assert_eq!(cat, None);
    }
}
